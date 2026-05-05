/// Virtio Block Device Driver
///
/// Implement read/write blocks từ virtio-blk device trong QEMU.
///
/// Flow cho một read request:
///   1. Build 3-descriptor chain:
///      [0] Header (type=READ, sector=N) → readable by device
///      [1] Data buffer (512 bytes)      → writable by device
///      [2] Status byte                  → writable by device
///   2. Add chain head to Available Ring
///   3. Notify device (write queue index to QueueNotify)
///   4. Poll Used Ring cho response
///   5. Check status byte == VIRTIO_BLK_S_OK

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{fence, Ordering};
use spin::Mutex;

use super::pci::{find_device, enable_bus_mastering, PciDevice};
use super::virtio::*;

pub const SECTOR_SIZE: usize = 512;

// ---------------------------------------------------------------------------
// Block Cache
// ---------------------------------------------------------------------------

const CACHE_SIZE: usize = 16;

struct CacheEntry {
    sector: u64,
    data: [u8; SECTOR_SIZE],
    valid: bool,
    dirty: bool,
}

impl CacheEntry {
    const fn empty() -> Self {
        CacheEntry { sector: 0, data: [0u8; SECTOR_SIZE], valid: false, dirty: false }
    }
}

// ---------------------------------------------------------------------------
// Virtio Block Driver
// ---------------------------------------------------------------------------

/// Aligned virtqueue buffers — cần align 4096 cho PFN
#[repr(C, align(4096))]
struct VirtqueueBuffers {
    desc:  [VirtqDesc; VIRTQUEUE_SIZE],
    avail: VirtqAvail,
    _pad:  [u8; 4096],  // padding để used ring align 4096
    used:  VirtqUsed,
}

pub struct VirtioBlkDev {
    io_base:    u16,
    queue_buf:  Box<VirtqueueBuffers>,
    next_desc:  usize,
    last_used:  u16,
    num_sectors: u64,
    cache:      [CacheEntry; CACHE_SIZE],
    cache_next: usize,
}

impl VirtioBlkDev {
    /// Khởi tạo virtio-blk device
    pub fn new(pci_dev: &PciDevice) -> Option<Self> {
        let io_base = pci_dev.io_base();
        if io_base == 0 {
            crate::serial_println!("[virtio-blk] Invalid I/O base");
            return None;
        }

        crate::serial_println!("[virtio-blk] Initializing at I/O base {:#x}", io_base);

        // Enable bus mastering
        enable_bus_mastering(pci_dev);

        unsafe {
            // 1. Reset device
            pci_write_u8(io_base, VIRTIO_PCI_STATUS, 0);

            // 2. Acknowledge
            pci_write_u8(io_base, VIRTIO_PCI_STATUS, VIRTIO_STATUS_ACKNOWLEDGE);

            // 3. Driver loaded
            pci_write_u8(io_base, VIRTIO_PCI_STATUS,
                VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);

            // 4. Read features (we accept minimal)
            let features = pci_read_u32(io_base, VIRTIO_PCI_HOST_FEATURES);
            crate::serial_println!("[virtio-blk] Device features: {:#x}", features);

            // Accept no optional features for simplicity
            pci_write_u32(io_base, VIRTIO_PCI_GUEST_FEATURES, 0);

            // 5. Setup virtqueue 0
            pci_write_u16(io_base, VIRTIO_PCI_QUEUE_SEL, 0);
            let queue_size = pci_read_u16(io_base, VIRTIO_PCI_QUEUE_SIZE);
            crate::serial_println!("[virtio-blk] Queue size: {}", queue_size);

            if queue_size == 0 {
                crate::serial_println!("[virtio-blk] Queue size is 0, aborting");
                return None;
            }

            // Allocate aligned virtqueue buffers
            let queue_buf = Box::new(VirtqueueBuffers {
                desc: [VirtqDesc::default(); VIRTQUEUE_SIZE],
                avail: VirtqAvail { flags: 0, idx: 0, ring: [0; VIRTQUEUE_SIZE], used_event: 0 },
                _pad: [0; 4096],
                used: VirtqUsed {
                    flags: 0, idx: 0,
                    ring: [VirtqUsedElem::default(); VIRTQUEUE_SIZE],
                    avail_event: 0,
                },
            });

            // Tell device where the queue is (PFN = physical page number)
            let queue_phys = queue_buf.as_ref() as *const _ as u64;
            let pfn = (queue_phys / 4096) as u32;
            crate::serial_println!("[virtio-blk] Queue PFN: {:#x} (addr={:#x})", pfn, queue_phys);
            pci_write_u32(io_base, VIRTIO_PCI_QUEUE_PFN, pfn);

            // 6. Read disk geometry (num sectors)
            // Config space starts at offset VIRTIO_PCI_CONFIG_OFF
            // For block device: first 8 bytes = capacity (num sectors)
            let cap_lo = pci_read_u32(io_base, VIRTIO_PCI_CONFIG_OFF);
            let cap_hi = pci_read_u32(io_base, VIRTIO_PCI_CONFIG_OFF + 4);
            let num_sectors = (cap_lo as u64) | ((cap_hi as u64) << 32);
            crate::serial_println!("[virtio-blk] Disk capacity: {} sectors ({} MiB)",
                num_sectors, num_sectors * 512 / 1024 / 1024);

            // 7. Mark driver ready
            pci_write_u8(io_base, VIRTIO_PCI_STATUS,
                VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_DRIVER_OK);

            let status = pci_read_u8(io_base, VIRTIO_PCI_STATUS);
            crate::serial_println!("[virtio-blk] Status: {:#x}", status);

            Some(VirtioBlkDev {
                io_base,
                queue_buf,
                next_desc: 0,
                last_used: 0,
                num_sectors,
                cache: core::array::from_fn(|_| CacheEntry::empty()),
                cache_next: 0,
            })
        }
    }

    /// Đọc một sector (512 bytes) từ disk
    pub fn read_sector(&mut self, sector: u64, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
        // Check cache first
        for entry in &self.cache {
            if entry.valid && entry.sector == sector {
                buf.copy_from_slice(&entry.data);
                return Ok(());
            }
        }

        // Do actual read
        self.do_request(VIRTIO_BLK_T_IN, sector, buf)?;

        // Cache result
        let idx = self.cache_next % CACHE_SIZE;
        self.cache[idx] = CacheEntry {
            sector,
            data: *buf,
            valid: true,
            dirty: false,
        };
        self.cache_next += 1;

        Ok(())
    }

    /// Ghi một sector (512 bytes) ra disk
    pub fn write_sector(&mut self, sector: u64, buf: &[u8; SECTOR_SIZE]) -> Result<(), &'static str> {
        // Invalidate cache entry
        for entry in &mut self.cache {
            if entry.valid && entry.sector == sector {
                entry.valid = false;
            }
        }

        self.do_request(VIRTIO_BLK_T_OUT, sector, &mut buf.clone())
    }

    /// Thực hiện một I/O request qua virtqueue
    fn do_request(&mut self, req_type: u32, sector: u64, buf: &mut [u8; SECTOR_SIZE])
        -> Result<(), &'static str>
    {
        // Build request header và status buffer trên stack
        let header = VirtioBlkReqHeader {
            req_type,
            reserved: 0,
            sector,
        };
        let mut status: u8 = 0xFF; // Will be written by device

        let header_phys = &header as *const _ as u64;
        let buf_phys = buf.as_ptr() as u64;
        let status_phys = &status as *const _ as u64;

        let d = &mut self.queue_buf.desc;
        let base = self.next_desc % VIRTQUEUE_SIZE;
        let d0 = base;
        let d1 = (base + 1) % VIRTQUEUE_SIZE;
        let d2 = (base + 2) % VIRTQUEUE_SIZE;

        // Descriptor 0: header (readable by device)
        d[d0] = VirtqDesc {
            addr:  header_phys,
            len:   core::mem::size_of::<VirtioBlkReqHeader>() as u32,
            flags: VIRTQ_DESC_F_NEXT,
            next:  d1 as u16,
        };

        // Descriptor 1: data buffer
        let data_flags = if req_type == VIRTIO_BLK_T_IN {
            VIRTQ_DESC_F_WRITE | VIRTQ_DESC_F_NEXT // Device writes (read op)
        } else {
            VIRTQ_DESC_F_NEXT // Device reads (write op)
        };
        d[d1] = VirtqDesc {
            addr:  buf_phys,
            len:   SECTOR_SIZE as u32,
            flags: data_flags,
            next:  d2 as u16,
        };

        // Descriptor 2: status byte (device writes)
        d[d2] = VirtqDesc {
            addr:  status_phys,
            len:   1,
            flags: VIRTQ_DESC_F_WRITE,
            next:  0,
        };

        // Add to available ring
        let avail_idx = self.queue_buf.avail.idx as usize % VIRTQUEUE_SIZE;
        self.queue_buf.avail.ring[avail_idx] = d0 as u16;

        // Memory barrier before updating idx
        fence(Ordering::SeqCst);
        self.queue_buf.avail.idx = self.queue_buf.avail.idx.wrapping_add(1);
        fence(Ordering::SeqCst);

        // Notify device
        unsafe {
            pci_write_u16(self.io_base, VIRTIO_PCI_QUEUE_NOTIFY, 0);
        }

        // Poll used ring (busy wait — will be replaced with interrupt in future)
        let mut timeout = 1_000_000u32;
        loop {
            fence(Ordering::SeqCst);
            if self.queue_buf.used.idx != self.last_used {
                self.last_used = self.last_used.wrapping_add(1);
                break;
            }
            timeout -= 1;
            if timeout == 0 {
                crate::serial_println!("[virtio-blk] Timeout waiting for sector {}", sector);
                return Err("virtio-blk timeout");
            }
            core::hint::spin_loop();
        }

        self.next_desc = (self.next_desc + 3) % VIRTQUEUE_SIZE;

        if status != VIRTIO_BLK_S_OK {
            crate::serial_println!("[virtio-blk] I/O error: status={}", status);
            return Err("virtio-blk I/O error");
        }

        Ok(())
    }

    pub fn num_sectors(&self) -> u64 { self.num_sectors }
}

// ---------------------------------------------------------------------------
// Global block device
// ---------------------------------------------------------------------------

static BLOCK_DEV: Mutex<Option<VirtioBlkDev>> = Mutex::new(None);

/// Khởi tạo virtio-blk driver — scan PCI, init device
pub fn init() -> bool {
    use super::virtio::{VIRTIO_VENDOR_ID, VIRTIO_BLK_DEVICE_ID};

    crate::serial_println!("[virtio-blk] Scanning PCI bus...");

    match find_device(VIRTIO_VENDOR_ID, VIRTIO_BLK_DEVICE_ID) {
        Some(pci_dev) => {
            crate::serial_println!(
                "[virtio-blk] Found at {:02x}:{:02x}.{} I/O={:#x}",
                pci_dev.bus, pci_dev.dev, pci_dev.func, pci_dev.io_base()
            );
            match VirtioBlkDev::new(&pci_dev) {
                Some(dev) => {
                    *BLOCK_DEV.lock() = Some(dev);
                    crate::serial_println!("[virtio-blk] Driver initialized OK");
                    true
                }
                None => {
                    crate::serial_println!("[virtio-blk] Device init failed");
                    false
                }
            }
        }
        None => {
            crate::serial_println!("[virtio-blk] No device found (run QEMU with -drive if=virtio)");
            false
        }
    }
}

/// Read sector through global device
pub fn read_sector(sector: u64, buf: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    match BLOCK_DEV.lock().as_mut() {
        Some(dev) => dev.read_sector(sector, buf),
        None => Err("virtio-blk: device not initialized"),
    }
}

/// Write sector through global device
pub fn write_sector(sector: u64, buf: &[u8; SECTOR_SIZE]) -> Result<(), &'static str> {
    match BLOCK_DEV.lock().as_mut() {
        Some(dev) => dev.write_sector(sector, buf),
        None => Err("virtio-blk: device not initialized"),
    }
}

/// Get disk size in sectors
pub fn num_sectors() -> Option<u64> {
    BLOCK_DEV.lock().as_ref().map(|d| d.num_sectors())
}
