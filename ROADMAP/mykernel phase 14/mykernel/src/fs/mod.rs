pub mod devfs;
pub mod ramfs;
pub mod vfs;

pub use vfs::{mount, open, create, stat, readdir, mkdir, FsError, FsResult, OpenFlags, FileType};

/// Khởi tạo VFS: mount ramfs tại "/" và devfs tại "/dev"
pub fn init() {
    use alloc::sync::Arc;

    // Mount root ramfs
    let rootfs = ramfs::RamFs::new();

    // Tạo cấu trúc thư mục cơ bản
    rootfs.write_file("/etc/hostname", b"mykernel\n");
    rootfs.write_file("/etc/version", b"MyKernel v0.1.0 - Phase 14\n");
    rootfs.write_file("/etc/motd", b"Welcome to MyKernel!\nType 'help' for commands.\n");
    rootfs.write_file("/bin/hello", b"Hello from /bin/hello!\n");

    // Mount "/" -> ramfs
    vfs::mount("/", rootfs);

    // Mount "/dev" -> devfs  
    let dev = devfs::DevFs::new();
    vfs::mount("/dev", dev);

    crate::serial_println!("[vfs] Filesystem initialized");
    crate::serial_println!("[vfs] Mounted: / (ramfs), /dev (devfs)");
}
