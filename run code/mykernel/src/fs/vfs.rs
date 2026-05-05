/// Virtual Filesystem Switch (VFS)
///
/// VFS là abstraction layer cho mọi filesystem:
/// - Mỗi filesystem implement trait `FileSystem`
/// - Files/dirs được represent bằng `VNode`
/// - Syscalls (open/read/write/close) đi qua VFS
/// - VFS dispatch tới filesystem cụ thể dựa trên mount point
///
/// Mount table:
///   "/" → ramfs (rootfs)
///   "/dev" → devfs

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FsError {
    NotFound,
    PermissionDenied,
    NotADirectory,
    IsADirectory,
    FileExists,
    InvalidArgument,
    NoSpace,
    NotMounted,
    EndOfFile,
    Io,
}

pub type FsResult<T> = Result<T, FsError>;

// ---------------------------------------------------------------------------
// File types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    RegularFile,
    Directory,
    CharDevice,
    BlockDevice,
    Symlink,
}

// ---------------------------------------------------------------------------
// Stat — file metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Stat {
    pub file_type: FileType,
    pub size: u64,
    pub inode: u64,
}

// ---------------------------------------------------------------------------
// File trait — một file descriptor đang mở
// ---------------------------------------------------------------------------

pub trait File: Send + Sync {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize>;
    fn write(&mut self, buf: &[u8]) -> FsResult<usize>;
    fn seek(&mut self, offset: i64, whence: SeekWhence) -> FsResult<u64>;
    fn stat(&self) -> FsResult<Stat>;
    fn close(&mut self) {}
}

#[derive(Debug, Clone, Copy)]
pub enum SeekWhence {
    Set,   // Seek from beginning
    Cur,   // Seek from current position
    End,   // Seek from end
}

// ---------------------------------------------------------------------------
// FileSystem trait — một filesystem cụ thể
// ---------------------------------------------------------------------------

pub trait FileSystem: Send + Sync {
    /// Tên filesystem (vd: "ramfs", "devfs")
    fn name(&self) -> &str;

    /// Open một file tại path, trả về File object
    fn open(&self, path: &str, flags: OpenFlags) -> FsResult<Arc<Mutex<dyn File>>>;

    /// Tạo file mới
    fn create(&self, path: &str) -> FsResult<Arc<Mutex<dyn File>>>;

    /// Xóa file
    fn unlink(&self, path: &str) -> FsResult<()>;

    /// List directory
    fn readdir(&self, path: &str) -> FsResult<Vec<DirEntry>>;

    /// Tạo directory
    fn mkdir(&self, path: &str) -> FsResult<()>;

    /// Remove a file. Default: unsupported.
    fn remove(&self, _path: &str) -> FsResult<()> {
        Err(FsError::PermissionDenied)
    }

    /// Stat a path
    fn stat(&self, path: &str) -> FsResult<Stat>;
}

#[derive(Debug, Clone, Copy)]
pub struct OpenFlags(u32);

impl OpenFlags {
    pub const READ:   OpenFlags = OpenFlags(1);
    pub const WRITE:  OpenFlags = OpenFlags(2);
    pub const CREATE: OpenFlags = OpenFlags(4);
    pub const TRUNC:  OpenFlags = OpenFlags(8);
    pub const APPEND: OpenFlags = OpenFlags(16);

    pub fn readable(self) -> bool { self.0 & 1 != 0 }
    pub fn writable(self) -> bool { self.0 & 2 != 0 }
    pub fn create(self) -> bool { self.0 & 4 != 0 }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub file_type: FileType,
    pub size: u64,
}

// ---------------------------------------------------------------------------
// Mount table
// ---------------------------------------------------------------------------

struct MountPoint {
    path: String,
    fs: Arc<dyn FileSystem>,
}

static MOUNT_TABLE: Mutex<Vec<MountPoint>> = Mutex::new(Vec::new());

/// Mount một filesystem tại path
pub fn mount(path: &str, fs: Arc<dyn FileSystem>) {
    crate::serial_println!("[vfs] Mounting {} at {}", fs.name(), path);
    MOUNT_TABLE.lock().push(MountPoint {
        path: String::from(path),
        fs,
    });
}

/// Tìm filesystem cho path, trả về (fs, relative_path)
fn find_fs(path: &str) -> FsResult<(Arc<dyn FileSystem>, String)> {
    let mounts = MOUNT_TABLE.lock();

    // Tìm mount point dài nhất match với path
    let mut best_match: Option<(&MountPoint, &str)> = None;

    for mount in mounts.iter() {
        if path.starts_with(mount.path.as_str()) {
            let relative = if mount.path == "/" {
                path
            } else {
                &path[mount.path.len()..]
            };
            let relative = if relative.is_empty() { "/" } else { relative };

            if best_match.is_none()
                || mount.path.len() > best_match.unwrap().0.path.len()
            {
                best_match = Some((mount, relative));
            }
        }
    }

    match best_match {
        Some((mount, relative)) => Ok((Arc::clone(&mount.fs), String::from(relative))),
        None => Err(FsError::NotMounted),
    }
}

// ---------------------------------------------------------------------------
// VFS operations (public API)
// ---------------------------------------------------------------------------

/// Open file, trả về file descriptor number
pub fn open(path: &str, flags: OpenFlags) -> FsResult<Arc<Mutex<dyn File>>> {
    let (fs, relative) = find_fs(path)?;
    fs.open(&relative, flags)
}

/// Create file
pub fn create(path: &str) -> FsResult<Arc<Mutex<dyn File>>> {
    let (fs, relative) = find_fs(path)?;
    fs.create(&relative)
}

/// Stat a path
pub fn stat(path: &str) -> FsResult<Stat> {
    let (fs, relative) = find_fs(path)?;
    fs.stat(&relative)
}

/// Read directory
pub fn readdir(path: &str) -> FsResult<Vec<DirEntry>> {
    let (fs, relative) = find_fs(path)?;
    fs.readdir(&relative)
}

/// Mkdir
pub fn mkdir(path: &str) -> FsResult<()> {
    let (fs, relative) = find_fs(path)?;
    fs.mkdir(&relative)
}

// ---------------------------------------------------------------------------
// File Descriptor Table (per-process)
// ---------------------------------------------------------------------------

static NEXT_FD: AtomicU64 = AtomicU64::new(3); // 0,1,2 = stdin,stdout,stderr

pub struct FdTable {
    entries: Vec<Option<Arc<Mutex<dyn File>>>>,
}

impl FdTable {
    pub fn new() -> Self {
        let mut entries: Vec<Option<Arc<Mutex<dyn File>>>> = Vec::new();
        // fd 0,1,2: stdin/stdout/stderr (null for now)
        entries.push(None); // stdin
        entries.push(None); // stdout
        entries.push(None); // stderr
        FdTable { entries }
    }

    /// Add file, return fd number
    pub fn add(&mut self, file: Arc<Mutex<dyn File>>) -> u64 {
        // Find free slot
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(file);
                return i as u64;
            }
        }
        // No free slot, push new
        let fd = self.entries.len() as u64;
        self.entries.push(Some(file));
        fd
    }

    pub fn get(&self, fd: u64) -> Option<Arc<Mutex<dyn File>>> {
        self.entries.get(fd as usize)?.as_ref().map(Arc::clone)
    }

    pub fn close(&mut self, fd: u64) -> bool {
        if let Some(slot) = self.entries.get_mut(fd as usize) {
            *slot = None;
            true
        } else {
            false
        }
    }
}

/// Remove a file from the filesystem that owns `path`.
/// Returns FsError::NotFound if the path does not exist.
pub fn remove(path: &str) -> FsResult<()> {
    let mounts = MOUNT_TABLE.lock();
    // Find the deepest mount point that is a prefix of path.
    let mut best_idx: Option<usize> = None;
    let mut best_len = 0usize;
    for (i, mp) in mounts.iter().enumerate() {
        if path.starts_with(mp.path.as_str()) && mp.path.len() > best_len {
            best_len = mp.path.len();
            best_idx = Some(i);
        }
    }
    match best_idx {
        Some(i) => {
            let mp = &mounts[i];
            // Path relative to mount point
            let rel = if path == mp.path.as_str() {
                "/"
            } else {
                &path[mp.path.len()..]
            };
            mp.fs.remove(rel)
        }
        None => Err(FsError::NotFound),
    }
}
