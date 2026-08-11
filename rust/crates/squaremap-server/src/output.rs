use fs2::FileExt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};

#[derive(Debug)]
pub struct OutputRoot {
    root: PathBuf,
    #[cfg(unix)]
    root_dir: Arc<File>,
    #[cfg(windows)]
    root_dir: Arc<cap_std::fs::Dir>,
    owner_lock: Arc<File>,
    writes: Arc<Mutex<()>>,
}
impl Clone for OutputRoot {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            #[cfg(unix)]
            root_dir: Arc::clone(&self.root_dir),
            #[cfg(windows)]
            root_dir: Arc::clone(&self.root_dir),
            owner_lock: Arc::clone(&self.owner_lock),
            writes: Arc::clone(&self.writes),
        }
    }
}

impl OutputRoot {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let configured = root.as_ref();
        let root_dir = open_root_handle(configured)?;
        let display_root = if configured.is_absolute() {
            configured.to_owned()
        } else {
            std::env::current_dir()?.join(configured)
        };
        let root_dir = Arc::new(root_dir);
        validate_open_root(&root_dir)?;
        let owner_lock = Arc::new(open_owner_lock(&root_dir)?);
        owner_lock.try_lock_exclusive().map_err(|error| io::Error::new(io::ErrorKind::AlreadyExists, format!("output root already owned: {error}")))?;
        cleanup_stale_cap(&root_dir)?;
        Ok(Self { root: display_root, root_dir, owner_lock, writes: Arc::new(Mutex::new(())) })
    }

    pub fn path(&self) -> &Path { &self.root }

    pub fn atomic_write<P: AsRef<Path>>(&self, relative: P, bytes: &[u8]) -> io::Result<()> {
        let _guard = self.writes.lock().map_err(|_| io::Error::other("output lock poisoned"))?;
        let relative = validate_relative(relative.as_ref())?;
        #[cfg(unix)]
        {
            return self.atomic_write_unix(&relative, bytes);
        }
        #[cfg(windows)]
        {
            return self.atomic_write_windows(&relative, bytes);
        }
        #[cfg(all(not(unix), not(windows)))]
        {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "output confinement unsupported on this platform"));
        }
    }

    #[cfg(unix)]
    fn atomic_write_unix(&self, relative: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut components = relative.components().peekable();
        let mut parent = self.root_dir.try_clone()?;
        while let Some(Component::Normal(name)) = components.next() {
            if components.peek().is_none() {
                let target = name.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 path"))?;
                let temp = next_temp_name();
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
        {
            return self.open_file_unix(&relative);
        }
        #[cfg(windows)]
        {
            return self.open_file_windows(&relative);
        }
        #[cfg(all(not(unix), not(windows)))]
        {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "output confinement unsupported on this platform"));
        }
    }
    #[cfg(windows)]
    fn atomic_write_windows(&self, relative: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut components = relative.components().peekable();
        let mut parent = self.root_dir.try_clone()?;
        while let Some(Component::Normal(name)) = components.next() {
            if components.peek().is_none() {
                let target = name.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 path"))?;
                let temp = next_temp_name();
                let mut options = cap_std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                let mut file = parent.open_with(&temp, &options)?;
                let result = (|| { file.write_all(bytes)?; file.flush()?; file.sync_all()?; parent.rename(&temp, &parent, target)?; Ok(()) })();
                if result.is_err() { let _ = parent.remove_file(&temp); }
                return result;
            }
            let name = name.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 path"))?;
            parent = match parent.open_dir(name) {
                Ok(dir) => dir,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    parent.create_dir(name)?;
                    parent.open_dir(name)?
                }
                Err(error) => return Err(error),
            };
        }
        Err(io::Error::new(io::ErrorKind::InvalidInput, "empty path"))
    }

    #[cfg(windows)]
    fn open_file_windows(&self, relative: &Path) -> io::Result<Option<(File, fs::Metadata)>> {
        let mut components = relative.components().peekable();
        let mut parent = self.root_dir.try_clone()?;
        while let Some(Component::Normal(name)) = components.next() {
            let name = name.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 path"))?;
            if components.peek().is_some() {
                parent = match parent.open_dir(name) {
                    Ok(dir) => dir,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(error),
                };
            } else {
                let file = match parent.open(name) {
                    Ok(file) => file.into_std(),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(error),
                };
                let metadata = file.metadata()?;
                if !metadata.is_file() { return Ok(None); }
                return Ok(Some((file, metadata)));
            }
        }
        Ok(None)
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

}
#[cfg(unix)]
fn open_root_handle(root: &Path) -> io::Result<File> {
    let mut parent = if root.is_absolute() {
        let path = std::ffi::CString::new("/").unwrap();
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        unsafe { File::from_raw_fd(fd) }
    } else {
        let path = std::ffi::CString::new(".").unwrap();
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        unsafe { File::from_raw_fd(fd) }
    };
    let mut saw_component = false;
    for component in root.components() {
        let Component::Normal(name) = component else {
            match component {
                Component::RootDir | Component::CurDir => continue,
                Component::ParentDir | Component::Prefix(_) => return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe output root")),
                Component::Normal(_) => unreachable!(),
            }
        };
        saw_component = true;
        let name = std::ffi::CString::new(name.as_encoded_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL root"))?;
        let mut fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::NotFound { return Err(error); }
            if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o755) } != 0 {
                let mkdir_error = io::Error::last_os_error();
                if mkdir_error.kind() != io::ErrorKind::AlreadyExists { return Err(mkdir_error); }
            }
            fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        }
        if fd < 0 { return Err(io::Error::last_os_error()); }
        parent = unsafe { File::from_raw_fd(fd) };
    }
    if !saw_component && root.as_os_str().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty output root"));
    }
    Ok(parent)
}
#[cfg(unix)]
fn validate_open_root(root: &File) -> io::Result<()> {
    let metadata = root.metadata()?;
    if !metadata.is_dir() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "output root is not a directory")); }
    Ok(())
}

#[cfg(windows)]
fn validate_open_root(root: &cap_std::fs::Dir) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(root.as_raw_handle() as _, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "output root is not a real directory"));
    }
    Ok(())
}

#[cfg(unix)]
fn open_owner_lock(root: &File) -> io::Result<File> {
    let name = std::ffi::CString::new(".squaremap-owner.lock").unwrap();
    let fd = unsafe { libc::openat(root.as_raw_fd(), name.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW, 0o600) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(windows)]
fn open_owner_lock(root: &cap_std::fs::Dir) -> io::Result<File> {
    use cap_std::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    let file = root.open_with(".squaremap-owner.lock", &options)?.into_std();
    validate_windows_regular_file(&file)?;
    Ok(file)
}

#[cfg(windows)]
fn validate_windows_regular_file(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "owner lock is not a regular file"));
    }
    Ok(())
}

#[cfg(unix)]
fn cleanup_stale_cap(root: &File) -> io::Result<()> {
    cleanup_stale_dir(&cap_std::fs::Dir::from_std_file(root.try_clone()?))
}

#[cfg(windows)]
fn cleanup_stale_cap(root: &cap_std::fs::Dir) -> io::Result<()> {
    cleanup_stale_dir(root)
}

fn cleanup_stale_dir(directory: &cap_std::fs::Dir) -> io::Result<()> {
    for entry in directory.read_dir(".")? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() { continue; }
        if file_type.is_dir() {
            cleanup_stale_dir(&entry.open_dir()?)?;
        } else if is_temp_name(&entry.file_name().to_string_lossy()) {
            match entry.remove_file() {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn open_windows_directory(parent: &cap_std::fs::Dir, name: &std::ffi::OsStr) -> io::Result<cap_std::fs::Dir> {
    use cap_std::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT};
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let file = parent.open_with(name, &options)?.into_std();
    validate_windows_directory_handle(&file)?;
    Ok(cap_std::fs::Dir::from_std_file(file))
}

#[cfg(windows)]
fn open_root_handle(root: &Path) -> io::Result<cap_std::fs::Dir> {
    use std::path::Prefix;
    if root.as_os_str().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty output root"));
    }
    let mut components = root.components().peekable();
    let mut parent = if root.is_absolute() {
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "absolute root missing volume"));
        };
        let mut anchor = PathBuf::from(prefix.as_os_str());
        if matches!(prefix.kind(), Prefix::Disk(_)) {
            anchor.push("\\");
        }
        cap_std::fs::Dir::open_ambient_dir(anchor, cap_std::ambient_authority())?
    } else {
        if components.peek().is_some_and(|component| matches!(component, Component::Prefix(_) | Component::RootDir)) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe relative output root"));
        }
        cap_std::fs::Dir::open_ambient_dir(".", cap_std::ambient_authority())?
    };
    for component in components {
        let Component::Normal(name) = component else {
            match component {
                Component::RootDir | Component::CurDir => continue,
                Component::ParentDir | Component::Prefix(_) => return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe output root")),
                Component::Normal(_) => unreachable!(),
            }
        };
        parent = match open_windows_directory(&parent, name) {
            Ok(dir) => dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                parent.create_dir(name)?;
                open_windows_directory(&parent, name)?
            }
            Err(error) => return Err(error),
        };
    }
    validate_open_root(&parent)?;
    Ok(parent)
}

#[cfg(windows)]
fn validate_windows_directory_handle(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "output root component is not a real directory"));
    }
    Ok(())
}

static LAST_TEMP_STAMP: AtomicU64 = AtomicU64::new(0);

fn next_temp_name() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().min(u64::MAX as u128) as u64;
    let previous = LAST_TEMP_STAMP.fetch_max(now, Ordering::Relaxed);
    let stamp = previous.saturating_add(1).max(now);
    LAST_TEMP_STAMP.store(stamp, Ordering::Relaxed);
    format!(".squaremap-tmp-{}-{}", std::process::id(), stamp)
}

#[cfg(all(not(unix), not(windows)))]
fn open_root_handle(root: &Path) -> io::Result<File> { File::open(root) }

fn is_temp_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(".squaremap-tmp-") else { return false };
    let mut fields = suffix.split('-');
    let Some(pid) = fields.next() else { return false };
    let Some(stamp) = fields.next() else { return false };
    fields.next().is_none() && !pid.is_empty() && !stamp.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()) && stamp.bytes().all(|byte| byte.is_ascii_digit())
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
            Component::Normal(name) if name != "" && name != ".squaremap-owner.lock" && !name.to_string_lossy().starts_with(".squaremap-") => clean.push(name),
            Component::Prefix(_) | Component::RootDir | Component::ParentDir | Component::CurDir => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "path escapes output root"));
            }
            Component::Normal(_) => return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid path")),
        }
    }
    if clean.as_os_str().is_empty() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty path")); }
    Ok(clean)
}
pub(crate) fn etag_for(metadata: &fs::Metadata) -> String {
    let nanos = metadata.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|duration| duration.as_nanos()).unwrap_or(0);
    format!("\"{:x}-{:x}\"", nanos, metadata.len())
}
