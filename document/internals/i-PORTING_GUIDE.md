# Porting Guide — Real Hardware và Beyond QEMU

> Hướng dẫn những gì cần thay đổi khi chạy MyKernel trên real hardware, switch từ BIOS sang UEFI, hay thêm support cho hardware mới.

---

## Real Hardware — Những Gì Cần Thay Đổi

### 1. Bootloader

**Hiện tại:** `bootloader v0.9.34` — BIOS-only, x86-only, rất opinionated về memory layout.

**Trên real hardware:**

```
Nếu mainboard hỗ trợ Legacy BIOS:
  bootloader crate hoạt động nếu:
  - Ghi kernel image ra USB drive (raw sectors)
  - Không có Secure Boot (cần disable trong BIOS)
  
  dd if=target/.../bootimage-mykernel.bin of=/dev/sdX
```

**Vấn đề phổ biến:**

```
1. Memory map khác:
   QEMU có memory map đơn giản, predictable.
   Real hardware có nhiều ACPI reserved regions, SMM regions.
   → BootInfoFrameAllocator cần filter kỹ hơn.

2. APIC base address khác:
   QEMU: LAPIC tại 0xFEE00000 (fixed)
   Real hardware: đọc từ IA32_APIC_BASE MSR:
   
   let apic_base: u64;
   unsafe {
       core::arch::asm!(
           "rdmsr",
           in("ecx") 0x1Bu32,  // IA32_APIC_BASE MSR
           out("eax") apic_base,
           out("edx") _,
       );
   }
   let base = apic_base & 0xFFFFF000;

3. ACPI tables tại địa chỉ khác:
   QEMU để RSDP tại địa chỉ cố định.
   Real hardware: scan 0xE0000-0xFFFFF và EBDA.

4. IOAPIC address từ ACPI:
   Không hardcode 0xFEC00000 — đọc từ MADT.

5. SMP: có thể có nhiều packages, NUMA nodes:
   QEMU simulates flat SMP.
   Real hardware cần topology-aware init.
```

### 2. Keyboard

**Hiện tại:** PS/2 keyboard qua port 0x60.

**Trên real hardware:**

```
Nhiều modern system không có PS/2 controller.
Cần USB HID driver.

Alternatively: dùng serial console (UART) thay vì keyboard.
MyKernel đã support serial stdio — chỉ cần kết nối UART.

Typical UART setup (COM1 = 0x3F8):
  - Speed: 115200 baud
  - Format: 8N1 (8 data bits, no parity, 1 stop bit)
  - Flow control: none (hoặc RTS/CTS)

Real hardware UART thường cần:
  - Kiểm tra FIFO size
  - Handle interrupt (thay vì polling)
```

### 3. VGA Output

**Hiện tại:** VGA text mode qua port 0x3D4/0x3D5 và framebuffer 0xB8000.

**Trên real hardware:**

```
VGA text mode không available trên:
  - UEFI systems (thường)
  - Systems với modern GPU (DisplayPort only)
  - Headless servers

Alternatives:
  1. Serial console (recommended cho servers)
  2. VESA/VBE framebuffer mode (cần bootloader support)
  3. GOP (Graphics Output Protocol) nếu dùng UEFI
  
Đơn giản nhất: redirect tất cả output ra serial:
  Đã done trong vga_buffer.rs (_print mirrors to serial)
  → Chỉ cần kết nối serial cable hoặc USB-serial adapter
```

### 4. Storage

**Hiện tại:** virtio-blk (QEMU only).

**Trên real hardware cần:**

```
NVMe (phổ biến nhất):
  - PCIe BAR0 MMIO interface
  - Admin Queue + I/O Queue
  - Command format: 64 bytes
  - Complex capability structure
  
AHCI (SATA SSD/HDD):
  - PCIe/PCI MMIO
  - Port-multiplied interface
  - ATA command set
  
USB Mass Storage:
  - USB EHCI/XHCI controller
  - Bulk-only transport
  - SCSI commands

Recommended starting point: NVMe (simpler than AHCI)
```

### 5. Network

**Hiện tại:** virtio-net (QEMU only).

**Trên real hardware:**

```
Intel e1000/e1000e (phổ biến nhất):
  PCI Device ID: 0x100E (e1000), 0x10D3 (82574L)
  
  Key registers (MMIO):
  CTRL  = 0x0000  // Device Control
  STATUS= 0x0008  // Device Status
  EERD  = 0x0014  // EEPROM Read (để đọc MAC)
  RCTL  = 0x0100  // Receive Control
  TCTL  = 0x0400  // Transmit Control
  RDBAL = 0x2800  // RX Descriptor Base Low
  TDBAL = 0x3800  // TX Descriptor Base Low
  
  Đây là driver phức tạp hơn virtio đáng kể.
  
Realtek RTL8139/RTL8169:
  Simpler than e1000
  Popular in older hardware
  
Recommended: Implement e1000 dựa trên Intel datasheet
  https://www.intel.com/content/dam/doc/manual/pci-pci-x-family-gbe-controllers-software-dev-manual.pdf
```

---

## UEFI Boot

UEFI khác BIOS đáng kể — không thể dùng `bootloader v0.9` trực tiếp.

### Options

**Option 1: UEFI Bootloader crate (thay thế)**

```toml
# Cargo.toml
[dependencies]
bootloader_api = "0.11"

[package.metadata.bootloader]
# Physical memory mappings, higher half...
```

`bootloader v0.11+` support UEFI. Nhưng API khác v0.9 — cần refactor `kernel_main`.

**Option 2: UEFI Application trực tiếp**

```rust
#![no_std]
#![no_main]

use uefi::prelude::*;
use uefi::proto::console::text::Output;

#[entry]
fn main(_image: Handle, mut system_table: SystemTable<Boot>) -> Status {
    // UEFI boot services available
    let stdout = system_table.stdout();
    stdout.output_string(cstr16!("MyKernel UEFI\n")).unwrap();

    // Exit boot services, get memory map
    let (_, memory_map) = system_table.exit_boot_services();

    // Jump to kernel main
    kernel_entry(&memory_map)
}
```

Cần thêm dependency: `uefi = "0.26"` (uefi-rs crate).

**Option 3: Chainload qua GRUB**

```
# grub.cfg
menuentry "MyKernel" {
    multiboot2 /boot/mykernel.bin
}
```

Sử dụng Multiboot2 protocol — bootloader v0.9 không support, cần viết custom entry point.

### UEFI Memory Map vs BIOS Memory Map

```
UEFI MemoryType (quan trọng):
  EfiConventionalMemory      = 7   → Usable RAM
  EfiBootServicesCode        = 3   → Sau exit_boot_services: usable
  EfiBootServicesData        = 4   → Sau exit_boot_services: usable
  EfiRuntimeServicesCode     = 5   → Giữ nguyên (UEFI runtime)
  EfiRuntimeServicesData     = 6   → Giữ nguyên (UEFI runtime)
  EfiACPIReclaimMemory       = 9   → Có thể dùng sau khi parse ACPI
  EfiMemoryMappedIO          = 11  → MMIO, không cấp phát
  EfiMemoryMappedIOPortSpace = 12  → Port I/O space
```

---

## Thêm Serial Console Interrupt Handler

Hiện tại shell dùng polling — tốt hơn là dùng interrupt:

```rust
// src/interrupts.rs
extern "x86-interrupt" fn serial_handler(_: InterruptStackFrame) {
    // Read all available bytes from UART FIFO
    loop {
        let lsr: u8 = unsafe {
            let v;
            core::arch::asm!("in al, dx", in("dx") 0x3FDu16, out("al") v);
            v
        };
        if lsr & 1 == 0 { break; }  // no more data

        let byte: u8 = unsafe {
            let v;
            core::arch::asm!("in al, dx", in("dx") 0x3F8u16, out("al") v);
            v
        };
        crate::drivers::serial::push_byte(byte);
    }
    unsafe { crate::apic::end_of_interrupt(); }
}

// src/interrupts.rs init_idt():
IDT[36].set_handler_fn(serial_handler);  // IRQ4 = COM1

// Enable UART interrupt:
unsafe {
    Port::<u8>::new(0x3F9).write(0x01);  // IER: enable received data interrupt
}

// Route IRQ4 via I/O APIC:
crate::apic::ioapic_route_irq(4, 36);
```

---

## KVM vs TCG — Performance và Features

```
QEMU TCG (default on Windows):
  Software emulation — slow
  SMEP/SMAP/RDRAND không available
  Good for compatibility testing

KVM (Linux host only):
  Hardware virtualization — fast
  Full CPU feature support (SMEP, SMAP, RDRAND hoạt động)
  Security features có thể test thật sự
  
  Run với KVM:
  qemu-system-x86_64 -enable-kvm [other flags]
  
  Security score sẽ tăng lên ~80-100/100 với KVM
```

---

## Cross-compilation cho non-x86

MyKernel hiện tại x86_64-only. Để port sang ARM64 (aarch64):

```
Những thứ cần thay:
  1. GDT/IDT → ARM Exception Level tables
  2. x86_64 crate → aarch64 crate
  3. Port I/O → MMIO only (ARM không có port I/O)
  4. APIC → GIC (Generic Interrupt Controller)
  5. TSC → ARM System Counter (CNTPCT_EL0)
  6. SYSCALL/SYSRET → SVC/ERET
  7. CR3 → TTBR0_EL1/TTBR1_EL1
  8. CPUID → MIDR_EL1, ID_AA64MMFR0_EL1

QEMU ARM64:
  qemu-system-aarch64 -machine virt -cpu cortex-a57
  Hỗ trợ virtio devices → virtio_blk/virtio_net ít cần thay đổi
  
Recommended custom target:
  aarch64-unknown-none-softfloat
```

---

## Checklist cho Real Hardware

- [ ] Disable Secure Boot trong UEFI
- [ ] Enable Legacy Boot hoặc switch sang UEFI bootloader
- [ ] Kiểm tra ACPI MADT có LAPIC entries không
- [ ] Đọc LAPIC base từ MSR thay vì hardcode
- [ ] Đọc IOAPIC base từ MADT thay vì hardcode
- [ ] Filter BIOS/UEFI reserved regions khỏi frame allocator
- [ ] Test với serial console (kết nối UART)
- [ ] Disable VGA nếu không có VGA output
- [ ] Implement real NIC driver (e1000 hoặc RTL8139)
- [ ] Handle ACPI quirks (có thể cần parse DSDT)
