pub mod pci;
pub mod virtio;
pub mod virtio_blk;

pub use virtio_blk::{read_sector, write_sector, num_sectors, SECTOR_SIZE};

/// Khởi tạo tất cả drivers
pub fn init() {
    crate::serial_println!("[drivers] Initializing...");

    // Khởi tạo virtio block driver
    if virtio_blk::init() {
        if let Some(n) = num_sectors() {
            crate::serial_println!("[drivers] virtio-blk: {} sectors ({} MiB)",
                n, n * 512 / 1024 / 1024);
        }
    } else {
        crate::serial_println!("[drivers] virtio-blk: not available");
    }
}
