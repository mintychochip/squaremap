use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct OutputRoot {
    root: PathBuf,
    writes: Arc<Mutex<()>>,
}

impl Clone for OutputRoot {
    fn clone(&self) -> Self { Self { root: self.root.clone(), writes: Arc::clone(&self.writes) } }
}

impl OutputRoot {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref();
        if let Ok(metadata) = fs::symlink_metadata(root) {
            if metadata.file_type().is_symlink() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "output root symlink"));
            }
        }
        fs::create_dir_all(root)?;
        let root = fs::canonicalize(root)?;
        let metadata = fs::symlink_metadata(&root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "output root is not a directory"));
        }
        Ok(Self { root, writes: Arc::new(Mutex::new(())) })
    }

    pub fn path(&self) -> &Path { &self.root }

    pub fn atomic_write<P: AsRef<Path>>(&self, relative: P, bytes: &[u8]) -> io::Result<()> {
        let _guard = self.writes.lock().map_err(|_| io::Error::other("output lock poisoned"))?;
        let relative = validate_relative(relative.as_ref())?;
        let target = self.prepare_parent(&relative)?;
        if let Ok(meta) = fs::symlink_metadata(&target) {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "target is not a regular file"));
            }
        }
        let file_name = target.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty target"))?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let temp_name = format!(".{}.squaremap-{}-{}", file_name.to_string_lossy(), std::process::id(), stamp);
        let temp = target.with_file_name(temp_name);
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            let mut file = options.open(&temp)?;
            file.write_all(bytes)?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            replace_file(&temp, &target)?;
            sync_parent(&target)?;
            Ok(())
        })();
        if result.is_err() { let _ = fs::remove_file(&temp); }
        result
    }

    pub(crate) fn open_file(&self, relative: &Path) -> io::Result<Option<(File, fs::Metadata)>> {
        let relative = validate_relative(relative)?;
        let path = self.root.join(&relative);
        let mut current = self.root.clone();
        for component in relative.components() {
            let Component::Normal(name) = component else { unreachable!() };
            current.push(name);
            let meta = match fs::symlink_metadata(&current) {
                Ok(meta) => meta,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
            if meta.file_type().is_symlink() { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "symlink path component")); }
            if current != path && !meta.is_dir() { return Ok(None); }
        }
        let file = match OpenOptions::new().read(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() { return Ok(None); }
        Ok(Some((file, metadata)))
    }

    fn prepare_parent(&self, relative: &Path) -> io::Result<PathBuf> {
        let mut current = self.root.clone();
        let mut components = relative.components().peekable();
        while let Some(Component::Normal(name)) = components.next() {
            current.push(name);
            if components.peek().is_some() {
                match fs::symlink_metadata(&current) {
                    Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid output parent"));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(current)
    }
}
pub(crate) fn validate_relative(path: &Path) -> io::Result<PathBuf> {
    if path.as_os_str().to_string_lossy().contains('\\') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "backslash path"));
    }
    if path.as_os_str().is_empty() || path.as_os_str().to_string_lossy().contains('\0') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid path"));
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) if name != "" => clean.push(name),
            Component::Prefix(_) | Component::RootDir | Component::ParentDir | Component::CurDir => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "path escapes output root"));
            }
            Component::Normal(_) => return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid path")),
        }
    }
    if clean.as_os_str().is_empty() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty path")); }
    Ok(clean)
}

fn replace_file(temp: &Path, target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        // Windows rename is not replace-existing. Remove only a previously validated regular target.
        if target.exists() { fs::remove_file(target)?; }
    }
    fs::rename(temp, target)
}

fn sync_parent(target: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(target.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
    }
    Ok(())

}
pub(crate) fn etag_for(metadata: &fs::Metadata) -> String {
    let nanos = metadata.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|duration| duration.as_nanos()).unwrap_or(0);
    format!("\"{:x}-{:x}\"", nanos, metadata.len())
}
