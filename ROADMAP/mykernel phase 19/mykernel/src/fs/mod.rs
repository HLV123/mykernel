pub mod devfs;
pub mod fat32;
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

/// Mount FAT32 image từ in-memory data
pub fn mount_fat32_image(image: alloc::vec::Vec<u8>, mount_point: &str) {
    use alloc::sync::Arc;
    let dev = fat32::RamBlockDevice::new(image);
    match fat32::Fat32Fs::new(dev) {
        Ok(fs) => {
            vfs::mount(mount_point, fs);
            crate::serial_println!("[fs] FAT32 mounted at {}", mount_point);
        }
        Err(e) => {
            crate::serial_println!("[fs] FAT32 mount failed: {:?}", e);
        }
    }
}
