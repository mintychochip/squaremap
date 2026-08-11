use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};

#[derive(Debug)]
pub struct OutputRoot {
    root: PathBuf,
    root_dir: Arc<File>,
    writes: Arc<Mutex<()>>,
}

impl Clone for OutputRoot {
    fn clone(&self) -> Self { Self { root: self.root.clone(), root_dir: Arc::clone(&self.root_dir), writes: Arc::clone(&self.writes) } }
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
        cleanup_stale(&root)?;
        let root_dir = Arc::new(open_root_handle(&root)?);
        Ok(Self { root, root_dir, writes: Arc::new(Mutex::new(())) })
    }

    pub fn path(&self) -> &Path { &self.root }

    pub fn atomic_write<P: AsRef<Path>>(&self, relative: P, bytes: &[u8]) -> io::Result<()> {
        let _guard = self.writes.lock().map_err(|_| io::Error::other("output lock poisoned"))?;
        let relative = validate_relative(relative.as_ref())?;
        #[cfg(unix)]
        { return self.atomic_write_unix(&relative, bytes); }
        #[cfg(not(unix))]
        let _ = (&relative, bytes);
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

    #[cfg(unix)]
    fn atomic_write_unix(&self, relative: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut components = relative.components().peekable();
        let mut parent = self.root_dir.try_clone()?;
        while let Some(Component::Normal(name)) = components.next() {
            if components.peek().is_none() {
                let target = name.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 path"))?;
                let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
                let temp = format!(".{}.squaremap-{}-{}", target, std::process::id(), stamp);
                let temp_c = std::ffi::CString::new(temp).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL path"))?;
                let target_c = std::ffi::CString::new(target).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL path"))?;
                let fd = unsafe { libc::openat(parent.as_raw_fd(), temp_c.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW, 0o600) };
                if fd < 0 { return Err(io::Error::last_os_error()); }
                let mut file = unsafe { File::from_raw_fd(fd) };
                let result = (|| { file.write_all(bytes)?; file.flush()?; file.sync_all()?; let rc = unsafe { libc::renameat(parent.as_raw_fd(), temp_c.as_ptr(), parent.as_raw_fd(), target_c.as_ptr()) }; if rc != 0 { return Err(io::Error::last_os_error()); } if unsafe { libc::fsync(parent.as_raw_fd()) } != 0 { return Err(io::Error::last_os_error()); } Ok(()) })();
                if result.is_err() { unsafe { libc::unlinkat(parent.as_raw_fd(), temp_c.as_ptr(), 0); } }
                return result;
            }
            let name = name.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 path"))?;
            let c = std::ffi::CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL path"))?;
            let mut fd = unsafe { libc::openat(parent.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW) };
            if fd < 0 && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
                if unsafe { libc::mkdirat(parent.as_raw_fd(), c.as_ptr(), 0o755) } != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists { return Err(io::Error::last_os_error()); }
                fd = unsafe { libc::openat(parent.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW) };
            }
            if fd < 0 { return Err(io::Error::last_os_error()); }
            parent = unsafe { File::from_raw_fd(fd) };
        }
        Err(io::Error::new(io::ErrorKind::InvalidInput, "empty path"))
    }
    pub(crate) fn open_file(&self, relative: &Path) -> io::Result<Option<(File, fs::Metadata)>> {
        let relative = validate_relative(relative)?;
        #[cfg(unix)]
        { return self.open_file_unix(&relative); }
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
    #[cfg(unix)]
    fn open_file_unix(&self, relative: &Path) -> io::Result<Option<(File, fs::Metadata)>> {
        let mut components = relative.components().peekable();
        let mut parent = self.root_dir.try_clone()?;
        while let Some(Component::Normal(name)) = components.next() {
            let name = name.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 path"))?;
            let c = std::ffi::CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL path"))?;
            if components.peek().is_some() {
                let fd = unsafe { libc::openat(parent.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW) };
                if fd < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::NotFound { return Ok(None); }
                    return Err(error);
                }
                parent = unsafe { File::from_raw_fd(fd) };
            } else {
                let fd = unsafe { libc::openat(parent.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW) };
                if fd < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::NotFound { return Ok(None); }
                    return Err(error);
                }
                let file = unsafe { File::from_raw_fd(fd) };
                let metadata = file.metadata()?;
                if !metadata.is_file() { return Ok(None); }
                return Ok(Some((file, metadata)));
            }
        }
        Ok(None)
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
#[cfg(unix)]
fn open_root_handle(root: &Path) -> io::Result<File> {
    let path = std::ffi::CString::new(root.as_os_str().as_encoded_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL root"))?;
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(not(unix))]
fn open_root_handle(root: &Path) -> io::Result<File> { File::open(root) }
fn cleanup_stale(directory: &Path) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() { continue; }
        if metadata.is_dir() {
            cleanup_stale(&path)?;
        } else if path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(".squaremap-")) {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
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
        use std::os::windows::ffi::OsStrExt;
        let temp: Vec<u16> = temp.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let result = unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileExW(temp.as_ptr(), target.as_ptr(), windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH) };
        if result == 0 { return Err(io::Error::last_os_error()); }
        return Ok(());
    }
    #[cfg(not(windows))]
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
