/// PCI Bus Enumeration
///
/// Scan PCI bus để tìm devices.
/// Dùng PCI Configuration Space (I/O port 0xCF8/0xCFC).
///
/// PCI Config Space Address format:
///   Bit 31: Enable bit
///   Bits 23:16: Bus number
///   Bits 15:11: Device number
///   Bits 10:8: Function number
///   Bits 7:2: Register offset
///   Bits 1:0: 0

use x86_64::instructions::port::Port;

const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA:    u16 = 0xCFC;

/// Đọc 4 bytes từ PCI Config Space
pub fn pci_config_read32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    let address: u32 = (1 << 31)
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC);

    unsafe {
        let mut addr_port: Port<u32> = Port::new(PCI_CONFIG_ADDRESS);
        let mut data_port: Port<u32> = Port::new(PCI_CONFIG_DATA);
        addr_port.write(address);
        data_port.read()
    }
}

/// Đọc 2 bytes từ PCI Config Space
pub fn pci_config_read16(bus: u8, dev: u8, func: u8, offset: u8) -> u16 {
    let val = pci_config_read32(bus, dev, func, offset & !3);
    ((val >> ((offset & 2) * 8)) & 0xFFFF) as u16
}

/// Ghi 4 bytes vào PCI Config Space
pub fn pci_config_write32(bus: u8, dev: u8, func: u8, offset: u8, val: u32) {
    let address: u32 = (1 << 31)
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC);

    unsafe {
        let mut addr_port: Port<u32> = Port::new(PCI_CONFIG_ADDRESS);
        let mut data_port: Port<u32> = Port::new(PCI_CONFIG_DATA);
        addr_port.write(address);
        data_port.write(val);
    }
}

/// PCI Device info
#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus:       u8,
    pub dev:       u8,
    pub func:      u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class:     u8,
    pub subclass:  u8,
    pub bar0:      u32,  // Base Address Register 0
    pub irq:       u8,
}

impl PciDevice {
    /// Đọc BAR0 (I/O base address cho legacy virtio)
    pub fn io_base(&self) -> u16 {
        // BAR0 bit 0 = 1 means I/O space
        if self.bar0 & 1 != 0 {
            (self.bar0 & !0x3) as u16
        } else {
            0
        }
    }
}

/// Scan toàn bộ PCI bus tìm devices
pub fn scan_pci_bus() -> alloc::vec::Vec<PciDevice> {
    let mut devices = alloc::vec::Vec::new();

    for bus in 0u8..=255 {
        for dev in 0u8..32 {
            for func in 0u8..8 {
                let vendor_device = pci_config_read32(bus, dev, func, 0);
                let vendor_id = (vendor_device & 0xFFFF) as u16;
                let device_id = ((vendor_device >> 16) & 0xFFFF) as u16;

                if vendor_id == 0xFFFF {
                    continue; // No device
                }

                let class_reg = pci_config_read32(bus, dev, func, 8);
                let class    = ((class_reg >> 24) & 0xFF) as u8;
                let subclass = ((class_reg >> 16) & 0xFF) as u8;

                let bar0 = pci_config_read32(bus, dev, func, 0x10);
                let irq_reg = pci_config_read32(bus, dev, func, 0x3C);
                let irq = (irq_reg & 0xFF) as u8;

                devices.push(PciDevice {
                    bus, dev, func,
                    vendor_id, device_id,
                    class, subclass,
                    bar0,
                    irq,
                });

                // If not multi-function, skip other functions
                let header = pci_config_read32(bus, dev, func, 0xC);
                let header_type = ((header >> 16) & 0x7F) as u8;
                if func == 0 && header_type & 0x80 == 0 {
                    break;
                }
            }
        }
        // Optimization: stop after bus 0 if no bridges found (simple systems)
        if bus == 0 { break; }
    }

    devices
}

/// Tìm device theo vendor + device ID
pub fn find_device(vendor_id: u16, device_id: u16) -> Option<PciDevice> {
    scan_pci_bus()
        .into_iter()
        .find(|d| d.vendor_id == vendor_id && d.device_id == device_id)
}

/// Enable PCI Bus Mastering (cần cho DMA)
pub fn enable_bus_mastering(dev: &PciDevice) {
    let cmd = pci_config_read16(dev.bus, dev.dev, dev.func, 4);
    pci_config_write32(dev.bus, dev.dev, dev.func, 4, (cmd | 0x4) as u32);
}
