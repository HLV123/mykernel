/// DevFS — device filesystem (/dev)
///
/// Cung cấp các device files:
///   /dev/null  — đọc EOF, write discard
///   /dev/zero  — đọc bytes 0, write discard  
///   /dev/serial — đọc/ghi serial port

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use super::vfs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, OpenFlags, SeekWhence, Stat,
};

// ---------------------------------------------------------------------------
// /dev/null
// ---------------------------------------------------------------------------

struct NullDevice;

impl File for NullDevice {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Err(FsError::EndOfFile)
    }
    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        Ok(buf.len()) // discard
    }
    fn seek(&mut self, _offset: i64, _whence: SeekWhence) -> FsResult<u64> {
        Ok(0)
    }
    fn stat(&self) -> FsResult<Stat> {
        Ok(Stat { file_type: FileType::CharDevice, size: 0, inode: 1 })
    }
}

// ---------------------------------------------------------------------------
// /dev/zero
// ---------------------------------------------------------------------------

struct ZeroDevice;

impl File for ZeroDevice {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        for b in buf.iter_mut() { *b = 0; }
        Ok(buf.len())
    }
    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        Ok(buf.len()) // discard
    }
    fn seek(&mut self, _offset: i64, _whence: SeekWhence) -> FsResult<u64> {
        Ok(0)
    }
    fn stat(&self) -> FsResult<Stat> {
        Ok(Stat { file_type: FileType::CharDevice, size: 0, inode: 2 })
    }
}

// ---------------------------------------------------------------------------
// /dev/serial — serial port output
// ---------------------------------------------------------------------------

struct SerialDevice;

impl File for SerialDevice {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Err(FsError::EndOfFile) // serial input not implemented yet
    }
    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        for &byte in buf {
            if byte.is_ascii() {
                crate::serial_print!("{}", byte as char);
            }
        }
        Ok(buf.len())
    }
    fn seek(&mut self, _offset: i64, _whence: SeekWhence) -> FsResult<u64> {
        Ok(0)
    }
    fn stat(&self) -> FsResult<Stat> {
        Ok(Stat { file_type: FileType::CharDevice, size: 0, inode: 3 })
    }
}

// ---------------------------------------------------------------------------
// DevFS
// ---------------------------------------------------------------------------

pub struct DevFs;

impl DevFs {
    pub fn new() -> Arc<Self> {
        Arc::new(DevFs)
    }
}

impl FileSystem for DevFs {
    fn name(&self) -> &str { "devfs" }

    fn open(&self, path: &str, _flags: OpenFlags) -> FsResult<Arc<Mutex<dyn File>>> {
        let path = path.trim_start_matches('/');
        match path {
            "null"   => Ok(Arc::new(Mutex::new(NullDevice))),
            "zero"   => Ok(Arc::new(Mutex::new(ZeroDevice))),
            "serial" => Ok(Arc::new(Mutex::new(SerialDevice))),
            _ => Err(FsError::NotFound),
        }
    }

    fn create(&self, _path: &str) -> FsResult<Arc<Mutex<dyn File>>> {
        Err(FsError::PermissionDenied)
    }

    fn unlink(&self, _path: &str) -> FsResult<()> {
        Err(FsError::PermissionDenied)
    }

    fn readdir(&self, path: &str) -> FsResult<Vec<DirEntry>> {
        if path == "/" || path.is_empty() {
            Ok(alloc::vec![
                DirEntry { name: String::from("null"),   file_type: FileType::CharDevice, size: 0 },
                DirEntry { name: String::from("zero"),   file_type: FileType::CharDevice, size: 0 },
                DirEntry { name: String::from("serial"), file_type: FileType::CharDevice, size: 0 },
            ])
        } else {
            Err(FsError::NotFound)
        }
    }

    fn mkdir(&self, _path: &str) -> FsResult<()> {
        Err(FsError::PermissionDenied)
    }

    fn stat(&self, path: &str) -> FsResult<Stat> {
        let path = path.trim_start_matches('/');
        match path {
            "" | "/" => Ok(Stat { file_type: FileType::Directory, size: 0, inode: 0 }),
            "null" | "zero" | "serial" =>
                Ok(Stat { file_type: FileType::CharDevice, size: 0, inode: 1 }),
            _ => Err(FsError::NotFound),
        }
    }
}
