pub mod devfs;
pub mod initramfs;
pub mod ramfs;
pub mod vfs;

pub use vfs::{mount, open, create, stat, readdir, mkdir, FsError, FsResult, OpenFlags, FileType};

pub fn init() {
    use alloc::sync::Arc;
    let rootfs = ramfs::RamFs::new();
    let rootfs_dyn: Arc<dyn vfs::FileSystem> = rootfs.clone();
    vfs::mount("/", rootfs_dyn);
    vfs::mount("/dev", devfs::DevFs::new());
    let cpio = initramfs::create_default_initramfs();
    initramfs::load_into_ramfs(&cpio, &rootfs);
    crate::serial_println!("[fs] VFS + initramfs ready");
}
