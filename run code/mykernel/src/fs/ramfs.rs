/// RamFS — in-memory filesystem
///
/// Lưu files trong RAM dưới dạng HashMap<path, Vec<u8>>
/// Đây là rootfs — mọi file tạo ra đều ở đây

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use super::vfs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, OpenFlags, SeekWhence, Stat,
};

// ---------------------------------------------------------------------------
// RamFile — một file đang mở
// ---------------------------------------------------------------------------

pub struct RamFile {
    data: Arc<Mutex<Vec<u8>>>,
    pos: usize,
    writable: bool,
}

impl File for RamFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let data = self.data.lock();
        let available = data.len().saturating_sub(self.pos);
        if available == 0 {
            return Err(FsError::EndOfFile);
        }
        let to_read = buf.len().min(available);
        buf[..to_read].copy_from_slice(&data[self.pos..self.pos + to_read]);
        self.pos += to_read;
        Ok(to_read)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        if !self.writable {
            return Err(FsError::PermissionDenied);
        }
        let mut data = self.data.lock();
        // Extend if needed
        if self.pos + buf.len() > data.len() {
            data.resize(self.pos + buf.len(), 0);
        }
        data[self.pos..self.pos + buf.len()].copy_from_slice(buf);
        self.pos += buf.len();
        Ok(buf.len())
    }

    fn seek(&mut self, offset: i64, whence: SeekWhence) -> FsResult<u64> {
        let data_len = self.data.lock().len() as i64;
        let new_pos = match whence {
            SeekWhence::Set => offset,
            SeekWhence::Cur => self.pos as i64 + offset,
            SeekWhence::End => data_len + offset,
        };
        if new_pos < 0 {
            return Err(FsError::InvalidArgument);
        }
        self.pos = new_pos as usize;
        Ok(self.pos as u64)
    }

    fn stat(&self) -> FsResult<Stat> {
        Ok(Stat {
            file_type: FileType::RegularFile,
            size: self.data.lock().len() as u64,
            inode: 0,
        })
    }
}

// ---------------------------------------------------------------------------
// RamFS
// ---------------------------------------------------------------------------

struct INode {
    data: Arc<Mutex<Vec<u8>>>,
    file_type: FileType,
}

pub struct RamFs {
    inodes: Mutex<BTreeMap<String, INode>>,
}

impl RamFs {
    pub fn new() -> Arc<Self> {
        let fs = RamFs {
            inodes: Mutex::new(BTreeMap::new()),
        };
        // Create root directory
        fs.inodes.lock().insert(
            String::from("/"),
            INode { data: Arc::new(Mutex::new(Vec::new())), file_type: FileType::Directory },
        );
        Arc::new(fs)
    }

    /// Tạo file với nội dung cho sẵn (dùng để populate rootfs)
    pub fn write_file(&self, path: &str, content: &[u8]) {
        let mut inodes = self.inodes.lock();
        let data = Arc::new(Mutex::new(content.to_vec()));
        inodes.insert(String::from(path), INode { data, file_type: FileType::RegularFile });
        crate::serial_println!("[ramfs] Created file {} ({} bytes)", path, content.len());
    }

    /// Helper: normalize path
    fn normalize(path: &str) -> String {
        if path.is_empty() { return String::from("/"); }
        if !path.starts_with('/') {
            let mut s = String::from("/");
            s.push_str(path);
            s
        } else {
            String::from(path)
        }
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &str { "ramfs" }

    fn open(&self, path: &str, flags: OpenFlags) -> FsResult<Arc<Mutex<dyn File>>> {
        let norm = Self::normalize(path);
        let inodes = self.inodes.lock();

        if let Some(inode) = inodes.get(&norm) {
            if inode.file_type == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            Ok(Arc::new(Mutex::new(RamFile {
                data: Arc::clone(&inode.data),
                pos: 0,
                writable: flags.writable(),
            })))
        } else if flags.create() {
            drop(inodes);
            return self.create(path);
        } else {
            Err(FsError::NotFound)
        }
    }

    fn create(&self, path: &str) -> FsResult<Arc<Mutex<dyn File>>> {
        let norm = Self::normalize(path);
        let data = Arc::new(Mutex::new(Vec::new()));
        self.inodes.lock().insert(
            norm,
            INode { data: Arc::clone(&data), file_type: FileType::RegularFile },
        );
        Ok(Arc::new(Mutex::new(RamFile { data, pos: 0, writable: true })))
    }

    fn unlink(&self, path: &str) -> FsResult<()> {
        let norm = Self::normalize(path);
        if self.inodes.lock().remove(&norm).is_some() {
            Ok(())
        } else {
            Err(FsError::NotFound)
        }
    }

    fn readdir(&self, path: &str) -> FsResult<Vec<DirEntry>> {
        let norm = Self::normalize(path);
        let inodes = self.inodes.lock();

        // Check path is a directory
        match inodes.get(&norm) {
            Some(inode) if inode.file_type == FileType::Directory => {}
            Some(_) => return Err(FsError::NotADirectory),
            None => return Err(FsError::NotFound),
        }

        let prefix = if norm == "/" { String::from("/") } else {
            let mut s = norm.clone(); s.push('/'); s
        };

        let mut entries = Vec::new();
        for (ipath, inode) in inodes.iter() {
            if ipath == &norm { continue; }
            if ipath.starts_with(prefix.as_str()) {
                let rest = &ipath[prefix.len()..];
                // Only direct children (no slash in rest)
                if !rest.contains('/') && !rest.is_empty() {
                    entries.push(DirEntry {
                        name: String::from(rest),
                        file_type: inode.file_type,
                        size: inode.data.lock().len() as u64,
                    });
                }
            }
        }
        Ok(entries)
    }

    fn mkdir(&self, path: &str) -> FsResult<()> {
        let norm = Self::normalize(path);
        let mut inodes = self.inodes.lock();
        if inodes.contains_key(&norm) {
            return Err(FsError::FileExists);
        }
        inodes.insert(norm, INode {
            data: Arc::new(Mutex::new(Vec::new())),
            file_type: FileType::Directory,
        });
        Ok(())
    }


    fn remove(&self, path: &str) -> crate::fs::vfs::FsResult<()> {
        let norm = Self::normalize(path);
        let mut inodes = self.inodes.lock();
        if inodes.remove(&norm).is_some() {
            Ok(())
        } else {
            Err(crate::fs::vfs::FsError::NotFound)
        }
    }

    fn stat(&self, path: &str) -> FsResult<Stat> {
        let norm = Self::normalize(path);
        let inodes = self.inodes.lock();
        match inodes.get(&norm) {
            Some(inode) => Ok(Stat {
                file_type: inode.file_type,
                size: inode.data.lock().len() as u64,
                inode: 0,
            }),
            None => Err(FsError::NotFound),
        }
    }
}
