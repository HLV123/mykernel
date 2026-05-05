pub mod pci;
pub mod virtio;
pub mod virtio_blk;
pub mod virtio_net;

pub use virtio_blk::{read_sector, write_sector, num_sectors, SECTOR_SIZE};
pub use virtio_net::{send_packet, recv_packet, get_mac, Packet};

pub fn init() {
    crate::serial_println!("[drivers] Initializing...");

    if virtio_blk::init() {
        if let Some(n) = num_sectors() {
            crate::serial_println!("[drivers] virtio-blk: {} sectors ({} MiB)",
                n, n * 512 / 1024 / 1024);
        }
    }

    if virtio_net::init() {
        if let Some(mac) = get_mac() {
            crate::serial_println!("[drivers] virtio-net: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
        }
    }
}
