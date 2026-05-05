// Filesystem module
//
// Exposes a thin, synchronous API over the Virtual Filesystem layer so that
// the shell and other kernel components can interact with files without
// dealing directly with Arc<Mutex<dyn File>> handles.
//
// On init the following hierarchy is created:
//   /         RamFS  (read-write in-memory filesystem)
//   /dev      DevFS  (/dev/null, /dev/zero, /dev/serial)
//   /         initramfs CPIO unpacked on top of RamFS
//              â†’ /bin/init, /bin/hello, /etc/hostname, /etc/motd,
//                /etc/os-release, /etc/shells, /README
//
// FAT32 volumes can be mounted at run-time via mount_fat32_image().

pub mod devfs;
pub mod fat32;
pub mod initramfs;
pub mod ramfs;
pub mod vfs;

// Re-export the most commonly used symbols so callers can write
// `use mykernel::fs::FsError` instead of `mykernel::fs::vfs::FsError`.
pub use vfs::{
    mount, open, create, stat, readdir, mkdir,
    FsError, FsResult, OpenFlags, FileType,
};

// ---------------------------------------------------------------------------
// Filesystem initialisation
// ---------------------------------------------------------------------------

/// Initialise the kernel VFS, mount the root RamFS and /dev DevFS, then
/// unpack the built-in initramfs CPIO archive into the root filesystem.
pub fn init() {
    use alloc::sync::Arc;

    // Mount a blank RamFS as the root filesystem.
    let rootfs = ramfs::RamFs::new();
    let rootfs_dyn: Arc<dyn vfs::FileSystem> = rootfs.clone();
    vfs::mount("/", rootfs_dyn);

    // Mount the device filesystem at /dev.
    vfs::mount("/dev", devfs::DevFs::new());

    // Unpack the CPIO initramfs archive into the root RamFS.
    // This populates /bin, /etc, /tmp, /proc, /usr, and several text files.
    let cpio = initramfs::create_default_initramfs();
    initramfs::load_into_ramfs(&cpio, &rootfs);

    crate::serial_println!("[fs] VFS + initramfs ready");
}

// ---------------------------------------------------------------------------
// Convenience helpers used by the shell
// ---------------------------------------------------------------------------

/// Read the entire contents of a file into a Vec<u8>.
///
/// Returns `FsError::NotFound` if the path does not exist.
pub fn read_file(path: &str) -> FsResult<alloc::vec::Vec<u8>> {

    let file = open(path, OpenFlags::READ)?;
    let size = {
        let info = {
            let x = match file.lock().stat() {
                Ok(s)  => s,
                Err(e) => return Err(e),
            };
            x
        };
        info.size
    };

    let mut buf = alloc::vec![0u8; size as usize];
    let n = {
        let x = match file.lock().read(&mut buf) {
            Ok(n)  => n,
            Err(e) => return Err(e),
        };
        x
    };
    buf.truncate(n);
    Ok(buf)
}

/// Write `data` to `path`, creating the file if it does not exist.
///
/// If the file already exists its previous contents are discarded.
pub fn write_file(path: &str, data: &[u8]) -> FsResult<()> {
    // Try to create; if it already exists just open for writing.
    let file = create(path).or_else(|_| open(path, OpenFlags::WRITE))?;
    let n = {
        let x = match file.lock().write(data) {
            Ok(n)  => n,
            Err(e) => return Err(e),
        };
        x
    };
    if n != data.len() {
        return Err(FsError::NotFound);
    }
    Ok(())
}

/// Remove a file from the VFS.
///
/// Currently delegates to `vfs::remove` which RamFS supports.
pub fn remove(path: &str) -> FsResult<()> {
    vfs::remove(path)
}

// ---------------------------------------------------------------------------
// FAT32 helper
// ---------------------------------------------------------------------------

/// Mount a FAT32 disk image (raw bytes) at `mount_point`.
///
/// Used during boot if a virtio-blk device was found and the first partition
/// contains a FAT32 volume.
pub fn mount_fat32_image(image: alloc::vec::Vec<u8>, mount_point: &str) {

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
