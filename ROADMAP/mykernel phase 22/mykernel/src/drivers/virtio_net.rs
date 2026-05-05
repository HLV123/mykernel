/// Virtio Network Device Driver
///
/// Virtio-net là network card ảo của QEMU.
/// Legacy PCI interface (device ID 0x1000).
///
/// Virtio-net có 3 virtqueues:
///   Queue 0: Receive Queue (RX) — device → driver
///   Queue 1: Transmit Queue (TX) — driver → device
///   Queue 2: Control Queue (optional)
///
/// Packet format:
///   [virtio_net_hdr (10 bytes)] [ethernet frame]
///
/// Ethernet frame:
///   [dst MAC 6b] [src MAC 6b] [ethertype 2b] [payload]
///
/// Flow gửi packet:
///   1. Điền virtio_net_hdr (flags=0, gso_type=NONE)
///   2. Điền ethernet frame
///   3. Add 2-descriptor chain vào TX virtqueue
///   4. Notify device
///
/// Flow nhận packet:
///   1. Pre-fill RX queue với receive buffers
///   2. Device DMA packet vào buffer khi có incoming
///   3. Driver poll used ring
///   4. Parse ethernet frame

use alloc::vec::Vec;
use alloc::collections::VecDeque;
use spin::Mutex;
use core::sync::atomic::{fence, Ordering};

use super::pci::{find_device, enable_bus_mastering, PciDevice};
use super::virtio::*;

// Virtio-net Device ID (legacy)
pub const VIRTIO_NET_DEVICE_ID: u16 = 0x1000;

// Virtio-net feature bits
pub const VIRTIO_NET_F_MAC:       u32 = 1 << 5;
pub const VIRTIO_NET_F_STATUS:    u32 = 1 << 16;
pub const VIRTIO_NET_F_MRG_RXBUF: u32 = 1 << 15;

// Virtio-net header flags
pub const VIRTIO_NET_HDR_F_NEEDS_CSUM: u8 = 1;
pub const VIRTIO_NET_HDR_GSO_NONE:     u8 = 0;

// Queue indices
pub const RX_QUEUE: u16 = 0;
pub const TX_QUEUE: u16 = 1;

// Buffer sizes
pub const RX_BUFFER_SIZE: usize = 1526; // MTU 1500 + virtio_net_hdr + eth overhead
pub const TX_BUFFER_SIZE: usize = 1526;
pub const RX_QUEUE_SIZE: usize = 64;
pub const TX_QUEUE_SIZE: usize = 64;

// Ethernet constants
pub const ETH_ALEN: usize = 6;
pub const ETH_HLEN: usize = 14; // 6+6+2
pub const ETH_P_IP:  u16 = 0x0800;
pub const ETH_P_ARP: u16 = 0x0806;
pub const ETH_P_IPV6:u16 = 0x86DD;

// ---------------------------------------------------------------------------
// Virtio-net header (10 bytes, precedes each packet)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtioNetHdr {
    pub flags:       u8,
    pub gso_type:    u8,
    pub hdr_len:     u16,
    pub gso_size:    u16,
    pub csum_start:  u16,
    pub csum_offset: u16,
    // num_buffers: u16 (only if VIRTIO_NET_F_MRG_RXBUF)
}

// ---------------------------------------------------------------------------
// Ethernet frame structures
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EthernetHdr {
    pub dst_mac: [u8; 6],
    pub src_mac: [u8; 6],
    pub ethertype: [u8; 2], // big-endian
}

impl EthernetHdr {
    pub fn ethertype(&self) -> u16 {
        ((self.ethertype[0] as u16) << 8) | self.ethertype[1] as u16
    }
}

/// Received packet (owned copy)
#[derive(Debug, Clone)]
pub struct Packet {
    pub data: Vec<u8>,  // includes virtio_net_hdr
}

impl Packet {
    pub fn eth_payload(&self) -> &[u8] {
        // Skip virtio_net_hdr (10 bytes) + eth header (14 bytes)
        let offset = core::mem::size_of::<VirtioNetHdr>() + ETH_HLEN;
        if self.data.len() > offset { &self.data[offset..] } else { &[] }
    }

    pub fn eth_hdr(&self) -> Option<&EthernetHdr> {
        let offset = core::mem::size_of::<VirtioNetHdr>();
        if self.data.len() >= offset + ETH_HLEN {
            Some(unsafe { &*(self.data[offset..].as_ptr() as *const EthernetHdr) })
        } else {
            None
        }
    }

    pub fn ethertype(&self) -> Option<u16> {
        self.eth_hdr().map(|h| h.ethertype())
    }
}

// ---------------------------------------------------------------------------
// Virtqueue pair (RX + TX)
// ---------------------------------------------------------------------------

#[repr(C, align(4096))]
struct RxQueueBufs {
    desc:  [VirtqDesc; RX_QUEUE_SIZE],
    avail: VirtqAvail,
    _pad:  [u8; 2048],
    used:  VirtqUsed,
    buffers: [[u8; RX_BUFFER_SIZE]; RX_QUEUE_SIZE],
}

#[repr(C, align(4096))]
struct TxQueueBufs {
    desc:  [VirtqDesc; TX_QUEUE_SIZE],
    avail: VirtqAvail,
    _pad:  [u8; 2048],
    used:  VirtqUsed,
    headers: [VirtioNetHdr; TX_QUEUE_SIZE / 2],
    buffers: [[u8; TX_BUFFER_SIZE]; TX_QUEUE_SIZE / 2],
}

// ---------------------------------------------------------------------------
// Virtio-net Driver
// ---------------------------------------------------------------------------

pub struct VirtioNetDev {
    io_base:      u16,
    mac:          [u8; 6],
    rx_last_used: u16,
    tx_next_desc: usize,
    tx_last_used: u16,
    rx_queue: alloc::boxed::Box<RxQueueBufs>,
    tx_queue: alloc::boxed::Box<TxQueueBufs>,
    pub rx_packets: VecDeque<Packet>,
}

impl VirtioNetDev {
    pub fn new(pci_dev: &PciDevice) -> Option<Self> {
        let io_base = pci_dev.io_base();
        if io_base == 0 {
            crate::serial_println!("[virtio-net] Invalid I/O base");
            return None;
        }

        crate::serial_println!("[virtio-net] Initializing at I/O={:#x}", io_base);
        enable_bus_mastering(pci_dev);

        unsafe {
            // Reset
            pci_write_u8(io_base, VIRTIO_PCI_STATUS, 0);
            // Acknowledge + Driver
            pci_write_u8(io_base, VIRTIO_PCI_STATUS,
                VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);

            // Read features
            let features = pci_read_u32(io_base, VIRTIO_PCI_HOST_FEATURES);
            crate::serial_println!("[virtio-net] Features: {:#x}", features);

            // Accept MAC feature only
            let guest_features = if features & VIRTIO_NET_F_MAC != 0 {
                VIRTIO_NET_F_MAC
            } else { 0 };
            pci_write_u32(io_base, VIRTIO_PCI_GUEST_FEATURES, guest_features);

            // Read MAC address (config space at offset 20 for legacy)
            let mut mac = [0u8; 6];
            for i in 0..6u16 {
                mac[i as usize] = pci_read_u8(io_base, VIRTIO_PCI_CONFIG_OFF + i);
            }
            crate::serial_println!("[virtio-net] MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

            // Allocate queues
            let mut rx_queue = alloc::boxed::Box::new(RxQueueBufs {
                desc: [VirtqDesc::default(); RX_QUEUE_SIZE],
                avail: VirtqAvail { flags: 0, idx: 0, ring: [0; VIRTQUEUE_SIZE], used_event: 0 },
                _pad: [0; 2048],
                used: VirtqUsed {
                    flags: 0, idx: 0,
                    ring: [VirtqUsedElem::default(); VIRTQUEUE_SIZE],
                    avail_event: 0,
                },
                buffers: [[0u8; RX_BUFFER_SIZE]; RX_QUEUE_SIZE],
            });

            let mut tx_queue = alloc::boxed::Box::new(TxQueueBufs {
                desc: [VirtqDesc::default(); TX_QUEUE_SIZE],
                avail: VirtqAvail { flags: 0, idx: 0, ring: [0; VIRTQUEUE_SIZE], used_event: 0 },
                _pad: [0; 2048],
                used: VirtqUsed {
                    flags: 0, idx: 0,
                    ring: [VirtqUsedElem::default(); VIRTQUEUE_SIZE],
                    avail_event: 0,
                },
                headers: [VirtioNetHdr::default(); TX_QUEUE_SIZE / 2],
                buffers: [[0u8; TX_BUFFER_SIZE]; TX_QUEUE_SIZE / 2],
            });

            // Setup RX queue (queue 0)
            pci_write_u16(io_base, VIRTIO_PCI_QUEUE_SEL, RX_QUEUE);
            let rx_pfn = (rx_queue.as_ref() as *const _ as u64 / 4096) as u32;
            pci_write_u32(io_base, VIRTIO_PCI_QUEUE_PFN, rx_pfn);

            // Pre-fill RX descriptors
            for i in 0..RX_QUEUE_SIZE {
                let buf_phys = &rx_queue.buffers[i] as *const _ as u64;
                rx_queue.desc[i] = VirtqDesc {
                    addr: buf_phys,
                    len: RX_BUFFER_SIZE as u32,
                    flags: VIRTQ_DESC_F_WRITE, // device writes
                    next: 0,
                };
                rx_queue.avail.ring[i] = i as u16;
            }
            rx_queue.avail.idx = RX_QUEUE_SIZE as u16;

            // Setup TX queue (queue 1)
            pci_write_u16(io_base, VIRTIO_PCI_QUEUE_SEL, TX_QUEUE);
            let tx_pfn = (tx_queue.as_ref() as *const _ as u64 / 4096) as u32;
            pci_write_u32(io_base, VIRTIO_PCI_QUEUE_PFN, tx_pfn);

            // Driver OK
            pci_write_u8(io_base, VIRTIO_PCI_STATUS,
                VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_DRIVER_OK);

            let status = pci_read_u8(io_base, VIRTIO_PCI_STATUS);
            crate::serial_println!("[virtio-net] Status: {:#x}", status);

            if status & VIRTIO_STATUS_FAILED as u8 != 0 {
                crate::serial_println!("[virtio-net] Device failed to initialize");
                return None;
            }

            Some(VirtioNetDev {
                io_base,
                mac,
                rx_last_used: 0,
                tx_next_desc: 0,
                tx_last_used: 0,
                rx_queue,
                tx_queue,
                rx_packets: VecDeque::new(),
            })
        }
    }

    pub fn mac(&self) -> [u8; 6] { self.mac }

    /// Send an ethernet frame
    /// data: ethernet frame (without virtio_net_hdr)
    pub fn send(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if data.len() > TX_BUFFER_SIZE - core::mem::size_of::<VirtioNetHdr>() {
            return Err("packet too large");
        }

        let hdr_idx = self.tx_next_desc % (TX_QUEUE_SIZE / 2);
        let buf_idx = hdr_idx;
        let desc_hdr = hdr_idx * 2;
        let desc_buf = desc_hdr + 1;

        if desc_buf >= TX_QUEUE_SIZE {
            return Err("TX queue full");
        }

        // Fill header
        self.tx_queue.headers[hdr_idx] = VirtioNetHdr {
            flags: 0,
            gso_type: VIRTIO_NET_HDR_GSO_NONE,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 0,
            csum_offset: 0,
        };

        // Copy data to TX buffer
        self.tx_queue.buffers[buf_idx][..data.len()].copy_from_slice(data);

        let hdr_phys = &self.tx_queue.headers[hdr_idx] as *const _ as u64;
        let buf_phys = &self.tx_queue.buffers[buf_idx] as *const _ as u64;

        // Descriptor chain: header → data
        self.tx_queue.desc[desc_hdr] = VirtqDesc {
            addr: hdr_phys,
            len: core::mem::size_of::<VirtioNetHdr>() as u32,
            flags: VIRTQ_DESC_F_NEXT,
            next: desc_buf as u16,
        };
        self.tx_queue.desc[desc_buf] = VirtqDesc {
            addr: buf_phys,
            len: data.len() as u32,
            flags: 0,
            next: 0,
        };

        // Add to available ring
        let avail_idx = self.tx_queue.avail.idx as usize % TX_QUEUE_SIZE;
        self.tx_queue.avail.ring[avail_idx] = desc_hdr as u16;

        fence(Ordering::SeqCst);
        self.tx_queue.avail.idx = self.tx_queue.avail.idx.wrapping_add(1);
        fence(Ordering::SeqCst);

        // Notify device (queue 1 = TX)
        unsafe { pci_write_u16(self.io_base, VIRTIO_PCI_QUEUE_NOTIFY, TX_QUEUE); }

        self.tx_next_desc += 1;
        crate::serial_println!("[virtio-net] TX: {} bytes", data.len());
        Ok(())
    }

    /// Poll for received packets
    pub fn poll_rx(&mut self) {
        loop {
            fence(Ordering::SeqCst);
            if self.rx_queue.used.idx == self.rx_last_used { break; }

            let idx = self.rx_last_used as usize % RX_QUEUE_SIZE;
            let used = self.rx_queue.used.ring[idx];
            let desc_idx = used.id as usize;
            let len = used.len as usize;

            if len > 0 && desc_idx < RX_QUEUE_SIZE {
                let data = self.rx_queue.buffers[desc_idx][..len.min(RX_BUFFER_SIZE)].to_vec();
                self.rx_packets.push_back(Packet { data });
                crate::serial_println!("[virtio-net] RX: {} bytes", len);
            }

            // Return descriptor to available ring
            let avail_idx = self.rx_queue.avail.idx as usize % RX_QUEUE_SIZE;
            self.rx_queue.avail.ring[avail_idx] = desc_idx as u16;
            self.rx_queue.avail.idx = self.rx_queue.avail.idx.wrapping_add(1);

            self.rx_last_used = self.rx_last_used.wrapping_add(1);
        }

        // Notify RX queue refill
        if !self.rx_packets.is_empty() {
            unsafe { pci_write_u16(self.io_base, VIRTIO_PCI_QUEUE_NOTIFY, RX_QUEUE); }
        }
    }

    pub fn recv(&mut self) -> Option<Packet> {
        self.poll_rx();
        self.rx_packets.pop_front()
    }
}

// ---------------------------------------------------------------------------
// Global network device
// ---------------------------------------------------------------------------

static NET_DEV: Mutex<Option<VirtioNetDev>> = Mutex::new(None);

pub fn init() -> bool {
    match find_device(VIRTIO_VENDOR_ID, VIRTIO_NET_DEVICE_ID) {
        Some(pci_dev) => {
            crate::serial_println!(
                "[virtio-net] Found at {:02x}:{:02x}.{} I/O={:#x}",
                pci_dev.bus, pci_dev.dev, pci_dev.func, pci_dev.io_base()
            );
            match VirtioNetDev::new(&pci_dev) {
                Some(dev) => {
                    *NET_DEV.lock() = Some(dev);
                    crate::serial_println!("[virtio-net] Driver ready");
                    true
                }
                None => false,
            }
        }
        None => {
            crate::serial_println!("[virtio-net] No device found");
            false
        }
    }
}

pub fn get_mac() -> Option<[u8; 6]> {
    NET_DEV.lock().as_ref().map(|d| d.mac())
}

pub fn send_packet(data: &[u8]) -> Result<(), &'static str> {
    match NET_DEV.lock().as_mut() {
        Some(dev) => dev.send(data),
        None => Err("no network device"),
    }
}

pub fn recv_packet() -> Option<Packet> {
    NET_DEV.lock().as_mut().and_then(|d| d.recv())
}

// ---------------------------------------------------------------------------
// Simple packet builder helpers
// ---------------------------------------------------------------------------

/// Build an ethernet frame
pub fn build_ethernet_frame(
    dst_mac: [u8; 6],
    src_mac: [u8; 6],
    ethertype: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(ETH_HLEN + payload.len());
    frame.extend_from_slice(&dst_mac);
    frame.extend_from_slice(&src_mac);
    frame.push((ethertype >> 8) as u8);
    frame.push((ethertype & 0xFF) as u8);
    frame.extend_from_slice(payload);
    frame
}

/// Build an ARP reply
/// RFC 826: ARP packet format
pub fn build_arp_reply(
    sender_mac: [u8; 6],
    sender_ip:  [u8; 4],
    target_mac: [u8; 6],
    target_ip:  [u8; 4],
) -> Vec<u8> {
    let mut arp = Vec::with_capacity(28);
    arp.extend_from_slice(&[0x00, 0x01]); // HTYPE: Ethernet
    arp.extend_from_slice(&[0x08, 0x00]); // PTYPE: IPv4
    arp.push(6);                           // HLEN
    arp.push(4);                           // PLEN
    arp.extend_from_slice(&[0x00, 0x02]); // OPER: Reply
    arp.extend_from_slice(&sender_mac);
    arp.extend_from_slice(&sender_ip);
    arp.extend_from_slice(&target_mac);
    arp.extend_from_slice(&target_ip);

    build_ethernet_frame(target_mac, sender_mac, ETH_P_ARP, &arp)
}

/// ICMP Echo Reply builder (for ping response)
pub fn build_icmp_echo_reply(
    dst_mac: [u8; 6],
    src_mac: [u8; 6],
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    id: u16,
    seq: u16,
    payload: &[u8],
) -> Vec<u8> {
    // ICMP Echo Reply
    let mut icmp = Vec::new();
    icmp.push(0u8); // Type 0 = Echo Reply
    icmp.push(0u8); // Code 0
    icmp.push(0u8); // Checksum (placeholder)
    icmp.push(0u8);
    icmp.push((id >> 8) as u8);
    icmp.push(id as u8);
    icmp.push((seq >> 8) as u8);
    icmp.push(seq as u8);
    icmp.extend_from_slice(payload);

    // Calculate ICMP checksum
    let cksum = internet_checksum(&icmp);
    icmp[2] = (cksum >> 8) as u8;
    icmp[3] = cksum as u8;

    // IPv4 header
    let total_len = 20 + icmp.len();
    let mut ip = Vec::new();
    ip.push(0x45); // Version=4, IHL=5
    ip.push(0);    // DSCP/ECN
    ip.push((total_len >> 8) as u8);
    ip.push(total_len as u8);
    ip.extend_from_slice(&[0, 1, 0, 0]); // ID, flags, fragment offset
    ip.push(64);   // TTL
    ip.push(1);    // Protocol: ICMP
    ip.push(0); ip.push(0); // Checksum placeholder
    ip.extend_from_slice(&src_ip);
    ip.extend_from_slice(&dst_ip);
    ip.extend_from_slice(&icmp);

    // Calculate IP checksum (header only, first 20 bytes)
    let ip_cksum = internet_checksum(&ip[..20]);
    ip[10] = (ip_cksum >> 8) as u8;
    ip[11] = ip_cksum as u8;

    build_ethernet_frame(dst_mac, src_mac, ETH_P_IP, &ip)
}

/// Internet checksum (RFC 1071)
pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | data[i+1] as u32;
        i += 2;
    }
    if data.len() % 2 != 0 {
        sum += (data[data.len()-1] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}
