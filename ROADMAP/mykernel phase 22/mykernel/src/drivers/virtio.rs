/// Virtio Device Base
///
/// Virtio là giao thức I/O ảo hóa chuẩn — QEMU implement phía host,
/// driver trong kernel implement phía guest.
///
/// Virtio over MMIO layout (dùng trong QEMU -device virtio-blk):
///   Offset 0x000: MagicValue (0x74726976 = "virt")
///   Offset 0x004: Version
///   Offset 0x008: DeviceID (2 = block device)
///   Offset 0x00c: VendorID
///   Offset 0x010: DeviceFeatures
///   Offset 0x014: DeviceFeaturesSel
///   Offset 0x020: DriverFeatures
///   Offset 0x024: DriverFeaturesSel
///   Offset 0x030: QueueSel
///   Offset 0x034: QueueNumMax
///   Offset 0x038: QueueNum
///   Offset 0x044: QueueReady
///   Offset 0x050: QueueNotify
///   Offset 0x060: InterruptStatus
///   Offset 0x064: InterruptACK
///   Offset 0x070: Status
///   Offset 0x080: QueueDescLow/High
///   Offset 0x090: QueueAvailLow/High
///   Offset 0x0a0: QueueUsedLow/High
///   Offset 0x0fc: ConfigGeneration
///   Offset 0x100: Config (device-specific)
///
/// NOTE: QEMU PCI virtio-blk dùng PCI config space thay vì MMIO.
/// Phase này implement virtio-blk qua PCI I/O ports (legacy interface).

// ---------------------------------------------------------------------------
// Virtio PCI (Legacy) definitions
// ---------------------------------------------------------------------------

/// Virtio PCI Vendor/Device IDs
pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
pub const VIRTIO_BLK_DEVICE_ID: u16 = 0x1001;  // Legacy block device

/// Virtio Status bits
pub const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
pub const VIRTIO_STATUS_DRIVER: u8 = 2;
pub const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
pub const VIRTIO_STATUS_FEATURES_OK: u8 = 8;
pub const VIRTIO_STATUS_FAILED: u8 = 128;

/// Virtio Feature bits
pub const VIRTIO_BLK_F_SIZE_MAX: u32 = 1 << 1;
pub const VIRTIO_BLK_F_SEG_MAX: u32 = 1 << 2;
pub const VIRTIO_BLK_F_GEOMETRY: u32 = 1 << 4;
pub const VIRTIO_BLK_F_RO: u32 = 1 << 5;
pub const VIRTIO_BLK_F_BLK_SIZE: u32 = 1 << 6;

/// Virtio Block request types
pub const VIRTIO_BLK_T_IN:  u32 = 0;  // Read
pub const VIRTIO_BLK_T_OUT: u32 = 1;  // Write

/// Virtio Block status
pub const VIRTIO_BLK_S_OK:     u8 = 0;
pub const VIRTIO_BLK_S_IOERR:  u8 = 1;
pub const VIRTIO_BLK_S_UNSUPP: u8 = 2;

/// Virtio Descriptor flags
pub const VIRTQ_DESC_F_NEXT:     u16 = 1;
pub const VIRTQ_DESC_F_WRITE:    u16 = 2; // Device writes to this buffer
pub const VIRTQ_DESC_F_INDIRECT: u16 = 4;

/// Virtio PCI Legacy I/O register offsets
pub const VIRTIO_PCI_HOST_FEATURES:   u16 = 0;
pub const VIRTIO_PCI_GUEST_FEATURES:  u16 = 4;
pub const VIRTIO_PCI_QUEUE_PFN:       u16 = 8;
pub const VIRTIO_PCI_QUEUE_SIZE:      u16 = 12;
pub const VIRTIO_PCI_QUEUE_SEL:       u16 = 14;
pub const VIRTIO_PCI_QUEUE_NOTIFY:    u16 = 16;
pub const VIRTIO_PCI_STATUS:          u16 = 18;
pub const VIRTIO_PCI_ISR:             u16 = 19;
pub const VIRTIO_PCI_CONFIG_OFF:      u16 = 20;  // Device config starts here

/// Virtqueue size (must be power of 2)
pub const VIRTQUEUE_SIZE: usize = 64;

// ---------------------------------------------------------------------------
// Virtqueue data structures
// ---------------------------------------------------------------------------

/// Virtqueue Descriptor — mô tả một buffer
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct VirtqDesc {
    pub addr:  u64,   // Physical address of buffer
    pub len:   u32,   // Length of buffer
    pub flags: u16,   // Flags (NEXT, WRITE, INDIRECT)
    pub next:  u16,   // Next descriptor index (if NEXT flag set)
}

/// Virtqueue Available Ring — driver → device
#[repr(C)]
pub struct VirtqAvail {
    pub flags: u16,
    pub idx:   u16,
    pub ring:  [u16; VIRTQUEUE_SIZE],
    pub used_event: u16,
}

/// Virtqueue Used Element
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct VirtqUsedElem {
    pub id:  u32,  // Descriptor chain head index
    pub len: u32,  // Bytes written by device
}

/// Virtqueue Used Ring — device → driver
#[repr(C)]
pub struct VirtqUsed {
    pub flags: u16,
    pub idx:   u16,
    pub ring:  [VirtqUsedElem; VIRTQUEUE_SIZE],
    pub avail_event: u16,
}

// ---------------------------------------------------------------------------
// Block device request header
// ---------------------------------------------------------------------------

/// Virtio Block Request Header (16 bytes)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtioBlkReqHeader {
    pub req_type: u32,   // VIRTIO_BLK_T_IN or VIRTIO_BLK_T_OUT
    pub reserved: u32,
    pub sector:   u64,   // Sector number (512 bytes per sector)
}

// ---------------------------------------------------------------------------
// PCI helpers (used by block driver)
// ---------------------------------------------------------------------------

use x86_64::instructions::port::Port;

/// Đọc 1 byte từ PCI I/O port
pub unsafe fn pci_read_u8(base: u16, offset: u16) -> u8 {
    let mut port: Port<u8> = Port::new(base + offset);
    port.read()
}

/// Đọc 2 bytes từ PCI I/O port
pub unsafe fn pci_read_u16(base: u16, offset: u16) -> u16 {
    let mut port: Port<u16> = Port::new(base + offset);
    port.read()
}

/// Đọc 4 bytes từ PCI I/O port
pub unsafe fn pci_read_u32(base: u16, offset: u16) -> u32 {
    let mut port: Port<u32> = Port::new(base + offset);
    port.read()
}

/// Ghi 1 byte ra PCI I/O port
pub unsafe fn pci_write_u8(base: u16, offset: u16, val: u8) {
    let mut port: Port<u8> = Port::new(base + offset);
    port.write(val);
}

/// Ghi 2 bytes ra PCI I/O port
pub unsafe fn pci_write_u16(base: u16, offset: u16, val: u16) {
    let mut port: Port<u16> = Port::new(base + offset);
    port.write(val);
}

/// Ghi 4 bytes ra PCI I/O port
pub unsafe fn pci_write_u32(base: u16, offset: u16, val: u32) {
    let mut port: Port<u32> = Port::new(base + offset);
    port.write(val);
}
