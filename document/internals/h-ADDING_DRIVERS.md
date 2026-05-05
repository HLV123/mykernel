# Thêm Driver Mới

> Hướng dẫn viết driver cho PCI device, virtio device, và filesystem mới.

---

## PCI Device Driver

### Bước 1: Identify Device

PCI device được nhận dạng bằng Vendor ID + Device ID:

```
Vendor ID 0x1AF4 = Red Hat (virtio devices)
  Device ID 0x1000 = virtio-net
  Device ID 0x1001 = virtio-blk
  Device ID 0x1002 = virtio-balloon
  Device ID 0x1003 = virtio-console
  Device ID 0x1050 = virtio-gpu

Vendor ID 0x8086 = Intel
  Device ID 0x100E = e1000 (82540EM Gigabit Ethernet)
  Device ID 0x2922 = ICH9 AHCI

Vendor ID 0x1234 = QEMU
  Device ID 0x1111 = QEMU VGA
```

Tìm Device ID trong QEMU documentation hoặc `lspci -n` trên Linux host.

### Bước 2: Thêm vào PCI Scanner

**File: `src/drivers/pci.rs`**

```rust
// Hàm scan hiện tại:
pub fn scan_for_device(vendor_id: u16, device_id: u16) -> Option<PciDevice> {
    for bus in 0..=255u8 {
        for device in 0..32u8 {
            for function in 0..8u8 {
                let vid = pci_read_config_u16(bus, device, function, 0x00);
                let did = pci_read_config_u16(bus, device, function, 0x02);
                if vid == vendor_id && did == device_id {
                    return Some(PciDevice { bus, device, function });
                }
            }
        }
    }
    None
}
```

### Bước 3: Đọc BARs (Base Address Registers)

```rust
pub struct PciDevice {
    pub bus:      u8,
    pub device:   u8,
    pub function: u8,
}

impl PciDevice {
    // BAR0-BAR5 tại config space offset 0x10-0x24
    pub fn read_bar(&self, bar_num: u8) -> u64 {
        let offset = 0x10 + bar_num as u16 * 4;
        let bar = pci_read_config_u32(self.bus, self.device, self.function, offset);

        if bar & 1 != 0 {
            // I/O BAR: bits 31:2 = I/O base address
            (bar & !0x3) as u64
        } else {
            // Memory BAR
            if (bar >> 1) & 3 == 2 {
                // 64-bit BAR: read next register too
                let bar_hi = pci_read_config_u32(self.bus, self.device, self.function, offset + 4);
                ((bar_hi as u64) << 32) | ((bar & !0xF) as u64)
            } else {
                (bar & !0xF) as u64
            }
        }
    }

    // Enable Bus Master (DMA), I/O Space, Memory Space
    pub fn enable(&self) {
        let cmd = pci_read_config_u16(self.bus, self.device, self.function, 0x04);
        pci_write_config_u16(self.bus, self.device, self.function, 0x04, cmd | 0x7);
    }
}
```

### Bước 4: Implement Driver Module

**File: `src/drivers/my_device.rs`**

```rust
use super::pci::{PciDevice, scan_for_device};
use x86_64::instructions::port::Port;

const MY_VENDOR_ID: u16 = 0x1234;
const MY_DEVICE_ID: u16 = 0x5678;

pub struct MyDevice {
    io_base: u16,
}

static MY_DEVICE: spin::Mutex<Option<MyDevice>> = spin::Mutex::new(None);

pub fn init() -> bool {
    let pci = match scan_for_device(MY_VENDOR_ID, MY_DEVICE_ID) {
        Some(p) => p,
        None => {
            crate::serial_println!("[my_device] not found");
            return false;
        }
    };

    pci.enable();

    let io_base = pci.read_bar(0) as u16;
    crate::serial_println!("[my_device] found at I/O base {:#x}", io_base);

    // Device-specific initialization
    let dev = MyDevice { io_base };
    dev.reset();
    dev.configure();

    *MY_DEVICE.lock() = Some(dev);
    true
}

impl MyDevice {
    fn reset(&self) {
        unsafe {
            let mut port = Port::<u8>::new(self.io_base + DEVICE_RESET_REG);
            port.write(1);
            // Wait for reset
            for _ in 0..1000 { core::hint::spin_loop(); }
        }
    }

    fn configure(&self) {
        // Device-specific setup
    }

    pub fn read_data(&self, buf: &mut [u8]) -> usize {
        // Implement based on device spec
        todo!()
    }

    pub fn write_data(&self, data: &[u8]) -> usize {
        // Implement based on device spec
        todo!()
    }
}

// Public API
pub fn read(buf: &mut [u8]) -> Option<usize> {
    MY_DEVICE.lock().as_ref()?.read_data(buf).into()
}
```

### Bước 5: Đăng ký trong `drivers/mod.rs`

```rust
pub mod my_device;  // thêm module

pub fn init() {
    crate::serial_println!("[drivers] Initializing...");

    if virtio_blk::init() { /* ... */ }
    if virtio_net::init() { /* ... */ }
    if my_device::init() {
        crate::serial_println!("[drivers] my_device ready");
    }
}
```

---

## Virtio Device Driver

Virtio devices có structure chung. Xem `src/drivers/virtio.rs` cho primitives.

### Virtio Device Initialization Sequence

```
1. Reset device:
   write 0 to VIRTIO_PCI_STATUS

2. Acknowledge device found:
   write VIRTIO_CONFIG_S_ACKNOWLEDGE to STATUS

3. Know how to drive it:
   write VIRTIO_CONFIG_S_DRIVER to STATUS

4. Feature negotiation:
   read VIRTIO_PCI_HOST_FEATURES (device offers)
   write subset to VIRTIO_PCI_GUEST_FEATURES

5. Setup virtqueues:
   for each queue:
     read max size from VIRTIO_PCI_QUEUE_NUM
     allocate aligned memory for virtqueue
     write physical address to VIRTIO_PCI_QUEUE_PFN

6. Driver ready:
   write VIRTIO_CONFIG_S_DRIVER_OK to STATUS
```

### Template cho Virtio Device

```rust
const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
const MY_VIRTIO_DEVICE_ID: u16 = 0x1003;  // ví dụ: virtio-console

// Virtio PCI legacy register offsets
const VIRTIO_PCI_HOST_FEATURES:  u16 = 0x00;
const VIRTIO_PCI_GUEST_FEATURES: u16 = 0x04;
const VIRTIO_PCI_QUEUE_PFN:      u16 = 0x08;
const VIRTIO_PCI_QUEUE_NUM:      u16 = 0x0C;
const VIRTIO_PCI_QUEUE_SEL:      u16 = 0x0E;
const VIRTIO_PCI_QUEUE_NOTIFY:   u16 = 0x10;
const VIRTIO_PCI_STATUS:         u16 = 0x12;
const VIRTIO_PCI_ISR:            u16 = 0x13;
// Device-specific config starts at 0x14

const VIRTIO_CONFIG_S_ACKNOWLEDGE: u8 = 1;
const VIRTIO_CONFIG_S_DRIVER:      u8 = 2;
const VIRTIO_CONFIG_S_DRIVER_OK:   u8 = 4;

pub struct MyVirtioDevice {
    io_base:    u16,
    rx_queue:   VirtQueue,
    tx_queue:   VirtQueue,
}

pub fn init() -> bool {
    let pci = match scan_for_device(VIRTIO_VENDOR_ID, MY_VIRTIO_DEVICE_ID) {
        Some(p) => p,
        None => return false,
    };
    pci.enable();

    let io_base = pci.read_bar(0) as u16;

    // Reset + acknowledge
    pci_write_status(io_base, 0);
    pci_write_status(io_base, VIRTIO_CONFIG_S_ACKNOWLEDGE | VIRTIO_CONFIG_S_DRIVER);

    // Feature negotiation
    let features = pci_read_features(io_base);
    pci_write_guest_features(io_base, features & MY_SUPPORTED_FEATURES);

    // Setup queues
    let rx_queue = setup_virtqueue(io_base, 0);  // queue 0 = RX
    let tx_queue = setup_virtqueue(io_base, 1);  // queue 1 = TX

    // Driver ready
    pci_write_status(io_base, VIRTIO_CONFIG_S_ACKNOWLEDGE
        | VIRTIO_CONFIG_S_DRIVER
        | VIRTIO_CONFIG_S_DRIVER_OK);

    let dev = MyVirtioDevice { io_base, rx_queue, tx_queue };
    *MY_DEVICE.lock() = Some(dev);
    true
}

fn setup_virtqueue(io_base: u16, queue_idx: u16) -> VirtQueue {
    unsafe {
        // Select queue
        Port::<u16>::new(io_base + VIRTIO_PCI_QUEUE_SEL).write(queue_idx);

        // Read max size
        let size = Port::<u16>::new(io_base + VIRTIO_PCI_QUEUE_NUM).read();

        // Allocate virtqueue (must be page-aligned)
        let vq = VirtQueue::new(size as usize);

        // Tell device about physical address
        let pfn = (vq.physical_addr() / 4096) as u32;
        Port::<u32>::new(io_base + VIRTIO_PCI_QUEUE_PFN).write(pfn);

        vq
    }
}
```

---

## FileSystem Driver

### Implement `FileSystem` Trait

```rust
use crate::fs::vfs::{
    FileSystem, File, FsError, FsResult, OpenFlags, DirEntry, Stat, FileType
};
use alloc::sync::Arc;
use spin::Mutex;

pub struct MyFs {
    // internal state
}

impl FileSystem for MyFs {
    fn open(&self, path: &str, flags: OpenFlags)
        -> FsResult<Arc<Mutex<dyn File>>>
    {
        // Find file at path
        // Return Arc<Mutex<MyFile>>
        todo!()
    }

    fn create(&self, path: &str)
        -> FsResult<Arc<Mutex<dyn File>>>
    {
        // Create new file
        todo!()
    }

    fn readdir(&self, path: &str) -> FsResult<Vec<DirEntry>> {
        // List directory contents
        // Return Vec<DirEntry { name, file_type, size }>
        todo!()
    }

    fn mkdir(&self, path: &str) -> FsResult<()> {
        todo!()
    }

    fn stat(&self, path: &str) -> FsResult<Stat> {
        // Return file metadata
        todo!()
    }

    fn remove(&self, path: &str) -> FsResult<()> {
        todo!()
    }
}
```

### Implement `File` Trait

```rust
pub struct MyFile {
    data:     Arc<Mutex<Vec<u8>>>,
    position: u64,
}

impl File for MyFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let data = self.data.lock();
        let pos = self.position as usize;

        if pos >= data.len() {
            return Err(FsError::EndOfFile);
        }

        let available = &data[pos..];
        let to_read = buf.len().min(available.len());
        buf[..to_read].copy_from_slice(&available[..to_read]);
        self.position += to_read as u64;
        Ok(to_read)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        let mut data = self.data.lock();
        let pos = self.position as usize;

        // Extend if needed
        if pos + buf.len() > data.len() {
            data.resize(pos + buf.len(), 0);
        }

        data[pos..pos + buf.len()].copy_from_slice(buf);
        self.position += buf.len() as u64;
        Ok(buf.len())
    }

    fn seek(&mut self, offset: i64, whence: crate::fs::vfs::SeekWhence) -> FsResult<u64> {
        use crate::fs::vfs::SeekWhence;
        let data_len = self.data.lock().len() as i64;

        let new_pos = match whence {
            SeekWhence::Start   => offset,
            SeekWhence::Current => self.position as i64 + offset,
            SeekWhence::End     => data_len + offset,
        };

        if new_pos < 0 { return Err(FsError::IoError); }
        self.position = new_pos as u64;
        Ok(self.position)
    }

    fn stat(&self) -> FsResult<Stat> {
        let size = self.data.lock().len() as u64;
        Ok(Stat {
            size,
            file_type: FileType::RegularFile,
        })
    }
}
```

### Mount Filesystem

```rust
// Trong kernel init hoặc khi detect storage:
pub fn mount_my_fs() {
    let fs = Arc::new(MyFs::new());
    crate::fs::vfs::mount("/myfs", fs);
    crate::serial_println!("[fs] MyFs mounted at /myfs");
}
```

---

## IRQ Handler cho Driver

Nếu driver cần interrupt-driven I/O:

```rust
// Trong src/interrupts.rs, thêm handler:
extern "x86-interrupt" fn my_device_handler(_: InterruptStackFrame) {
    crate::drivers::my_device::handle_interrupt();
    unsafe { crate::apic::end_of_interrupt(); }
}

// Đăng ký trong init_idt():
pub fn init_idt() {
    // ...
    IDT[MY_DEVICE_VECTOR].set_handler_fn(my_device_handler);
    // ...
}

// Route IRQ từ I/O APIC đến vector:
pub fn route_my_device_irq() {
    let irq_num = 9;  // IRQ9 thường available
    let vector  = 41; // MY_DEVICE_VECTOR
    crate::apic::ioapic_route_irq(irq_num, vector);
}
```

---

## Ví dụ: Virtio Console Driver

Virtio console (device 0x1003) — cung cấp serial-like interface:

```rust
pub struct VirtioConsole {
    io_base:    u16,
    rx_queue:   VirtQueue,  // queue 0: receiveq port0
    tx_queue:   VirtQueue,  // queue 1: transmitq port0
}

impl VirtioConsole {
    pub fn write(&mut self, data: &[u8]) {
        // Add data to TX queue
        let desc_idx = self.tx_queue.alloc_descriptor();
        self.tx_queue.set_descriptor(desc_idx, data.as_ptr() as u64, data.len() as u32, 0, 0);
        self.tx_queue.make_available(desc_idx);
        self.notify_device(1);  // notify transmitq
        self.wait_for_used();   // wait until device processes
    }

    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        // Poll receiveq used ring
        if let Some((desc_idx, len)) = self.rx_queue.poll_used() {
            let actual_len = len.min(buf.len() as u32) as usize;
            // Copy from descriptor buffer to buf
            unsafe {
                core::ptr::copy_nonoverlapping(
                    self.rx_queue.descriptor_addr(desc_idx) as *const u8,
                    buf.as_mut_ptr(),
                    actual_len,
                );
            }
            self.rx_queue.recycle(desc_idx);
            actual_len
        } else {
            0
        }
    }
}

// Implement FileSystem để mount tại /dev/console
impl File for VirtioConsole {
    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.write(buf);
        Ok(buf.len())
    }
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        Ok(self.read(buf))
    }
    // ...
}
```

---

## Checklist cho Driver mới

- [ ] Scan PCI bus để tìm device (Vendor ID + Device ID)
- [ ] Enable Bus Master trong PCI command register
- [ ] Đọc BAR để lấy I/O base hoặc MMIO base
- [ ] Reset device và negotiate features (virtio)
- [ ] Setup DMA buffers (page-aligned, physical address)
- [ ] Implement send/recv functions
- [ ] Đăng ký IRQ handler nếu cần interrupt-driven
- [ ] Expose public API qua static `Mutex<Option<Device>>`
- [ ] Gọi `init()` từ `src/drivers/mod.rs`
- [ ] Thêm device info vào boot banner (nếu relevant)
