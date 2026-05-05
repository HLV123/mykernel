#[allow(unused_imports)] use alloc::vec;
/// ELF64 Loader
///
/// Parse ELF64 binary, load PT_LOAD segments vào process address space,
/// setup initial stack với argc/argv, jump vào entry point trong ring 3.
///
/// ELF format tóm tắt:
/// - ELF Header (64 bytes): magic, type, arch, entry point, phoff
/// - Program Headers: mô tả các segments cần load
/// - PT_LOAD segments: code + data cần copy vào memory

use alloc::vec::Vec;
use x86_64::{
    structures::paging::{FrameAllocator, Page, PageTableFlags, Size4KiB},
    VirtAddr,
};
use crate::process::AddressSpace;

// ---------------------------------------------------------------------------
// ELF64 structures
// ---------------------------------------------------------------------------

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ET_EXEC: u16 = 2;   // Executable
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PT_NULL: u32 = 0;

// ELF flags
const PF_X: u32 = 1; // Execute
const PF_W: u32 = 2; // Write
const PF_R: u32 = 4; // Read

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct Elf64Header {
    e_ident:     [u8; 16],
    e_type:      u16,
    e_machine:   u16,
    e_version:   u32,
    e_entry:     u64,  // Entry point virtual address
    e_phoff:     u64,  // Program header table offset
    e_shoff:     u64,  // Section header table offset
    e_flags:     u32,
    e_ehsize:    u16,
    e_phentsize: u16,  // Size of each program header
    e_phnum:     u16,  // Number of program headers
    e_shentsize: u16,
    e_shnum:     u16,
    e_shstrndx:  u16,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct Elf64Phdr {
    p_type:   u32,   // Segment type
    p_flags:  u32,   // Segment flags (PF_R, PF_W, PF_X)
    p_offset: u64,   // Offset in file
    p_vaddr:  u64,   // Virtual address in memory
    p_paddr:  u64,   // Physical address (ignored)
    p_filesz: u64,   // Size in file
    p_memsz:  u64,   // Size in memory (>= filesz, rest zeroed)
    p_align:  u64,   // Alignment
}

// ---------------------------------------------------------------------------
// ELF error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ElfError {
    InvalidMagic,
    NotExecutable,
    NotX86_64,
    TooManySegments,
    SegmentOutOfBounds,
    MapFailed,
}

// ---------------------------------------------------------------------------
// ELF Loader
// ---------------------------------------------------------------------------

pub struct LoadedElf {
    pub entry_point: u64,
    pub stack_top: u64,
}

/// Load ELF binary vào address space
///
/// # Arguments
/// * `elf_data` - Raw ELF binary bytes
/// * `addr_space` - Target process address space
/// * `frame_allocator` - Physical frame allocator
/// * `phys_mem_offset` - Physical memory offset
pub fn load_elf(
    elf_data: &[u8],
    addr_space: &mut AddressSpace,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<LoadedElf, ElfError> {
    // Parse ELF header
    if elf_data.len() < core::mem::size_of::<Elf64Header>() {
        return Err(ElfError::InvalidMagic);
    }

    let header = unsafe { &*(elf_data.as_ptr() as *const Elf64Header) };

    // Validate magic
    if &header.e_ident[0..4] != &ELF_MAGIC {
        return Err(ElfError::InvalidMagic);
    }

    // Validate type and architecture
    if { header.e_type } != ET_EXEC {
        return Err(ElfError::NotExecutable);
    }
    if { header.e_machine } != EM_X86_64 {
        return Err(ElfError::NotX86_64);
    }

    let entry_point = { header.e_entry };
    let phoff = { header.e_phoff } as usize;
    let phnum = { header.e_phnum } as usize;
    let phentsize = { header.e_phentsize } as usize;

    crate::serial_println!("[elf] Entry: {:#x}, {} program headers", entry_point, phnum);

    // Load PT_LOAD segments
    for i in 0..phnum {
        let phdr_offset = phoff + i * phentsize;
        if phdr_offset + phentsize > elf_data.len() {
            return Err(ElfError::SegmentOutOfBounds);
        }

        let phdr = unsafe {
            &*(elf_data[phdr_offset..].as_ptr() as *const Elf64Phdr)
        };

        let p_type = { phdr.p_type };
        if p_type != PT_LOAD { continue; }

        let vaddr  = { phdr.p_vaddr };
        let filesz = { phdr.p_filesz } as usize;
        let memsz  = { phdr.p_memsz } as usize;
        let offset = { phdr.p_offset } as usize;
        let flags  = { phdr.p_flags };
        let align  = { phdr.p_align };

        crate::serial_println!(
            "[elf] PT_LOAD: vaddr={:#x} filesz={} memsz={} flags={:#x}",
            vaddr, filesz, memsz, flags
        );

        if offset + filesz > elf_data.len() {
            return Err(ElfError::SegmentOutOfBounds);
        }

        // Map pages for this segment
        let page_flags = elf_flags_to_page_flags(flags);
        load_segment(
            &elf_data[offset..offset + filesz],
            vaddr,
            filesz,
            memsz,
            page_flags,
            addr_space,
            frame_allocator,
            phys_mem_offset,
        )?;
    }

    // Setup user stack (16 KiB at 0x7fff_0000)
    let stack_top = setup_user_stack(addr_space, frame_allocator, phys_mem_offset)?;

    crate::serial_println!("[elf] Loaded OK, entry={:#x} stack={:#x}", entry_point, stack_top);

    Ok(LoadedElf { entry_point, stack_top })
}

/// Convert ELF segment flags to page table flags
fn elf_flags_to_page_flags(elf_flags: u32) -> PageTableFlags {
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if elf_flags & PF_W != 0 {
        flags |= PageTableFlags::WRITABLE;
    }
    // Note: NX flag would go here for non-execute pages
    flags
}

/// Load một segment vào address space
fn load_segment(
    data: &[u8],
    vaddr: u64,
    filesz: usize,
    memsz: usize,
    flags: PageTableFlags,
    addr_space: &mut AddressSpace,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), ElfError> {
    let page_size = 4096usize;

    // Calculate page range
    let start_page = vaddr & !(page_size as u64 - 1);
    let end_addr = vaddr + memsz as u64;
    let end_page = (end_addr + page_size as u64 - 1) & !(page_size as u64 - 1);

    let mut current_page = start_page;
    let mut data_offset = 0usize;

    while current_page < end_page {
        // Allocate physical frame
        let frame = frame_allocator.allocate_frame()
            .ok_or(ElfError::MapFailed)?;

        // Zero out the frame
        let frame_vaddr = phys_mem_offset + frame.start_address().as_u64();
        unsafe {
            core::ptr::write_bytes(frame_vaddr.as_mut_ptr::<u8>(), 0, page_size);
        }

        // Copy data into frame
        let page_vaddr_start = current_page;
        let copy_start = if page_vaddr_start < vaddr {
            // First page might have offset
            (vaddr - page_vaddr_start) as usize
        } else {
            0
        };

        let frame_dst = frame_vaddr.as_u64() + copy_start as u64;
        let remaining_data = filesz.saturating_sub(data_offset);
        let copy_len = (page_size - copy_start).min(remaining_data);

        if copy_len > 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data[data_offset..].as_ptr(),
                    frame_dst as *mut u8,
                    copy_len,
                );
            }
            data_offset += copy_len;
        }

        // Map frame into address space
        let page = Page::containing_address(VirtAddr::new(current_page));
        addr_space.map_page(page, frame, flags, frame_allocator, phys_mem_offset)
            .map_err(|_| ElfError::MapFailed)?;

        current_page += page_size as u64;
    }

    Ok(())
}

/// Setup user stack
fn setup_user_stack(
    addr_space: &mut AddressSpace,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<u64, ElfError> {
    let stack_top: u64 = 0x7fff_f000;
    let stack_pages = 4;
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

    for i in 0..stack_pages as u64 {
        let frame = frame_allocator.allocate_frame().ok_or(ElfError::MapFailed)?;
        let frame_vaddr = phys_mem_offset + frame.start_address().as_u64();
        unsafe { core::ptr::write_bytes(frame_vaddr.as_mut_ptr::<u8>(), 0, 4096); }

        let page_addr = stack_top - (i + 1) * 4096;
        let page = Page::containing_address(VirtAddr::new(page_addr));
        addr_space.map_page(page, frame, flags, frame_allocator, phys_mem_offset)
            .map_err(|_| ElfError::MapFailed)?;
    }

    Ok(stack_top)
}

// ---------------------------------------------------------------------------
// Embedded test ELF binary
// ---------------------------------------------------------------------------

/// Tạo minimal ELF64 binary trong memory để test loader
/// Binary này chỉ chạy syscall write + exit
pub fn create_test_elf() -> Vec<u8> {
    // x86_64 machine code: sys_write(1, msg, 25); sys_exit(0)
    // msg embedded at offset 0x1000 trong binary
    let code: &[u8] = &[
        // mov rax, 1 (SYS_WRITE)
        0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00,
        // mov rdi, 1 (stdout)
        0x48, 0xC7, 0xC7, 0x01, 0x00, 0x00, 0x00,
        // mov rsi, 0x401000 (message virtual address)
        0x48, 0xBE, 0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
        // mov rdx, 25 (length)
        0x48, 0xC7, 0xC2, 0x19, 0x00, 0x00, 0x00,
        // syscall
        0x0F, 0x05,
        // mov rax, 60 (SYS_EXIT)
        0x48, 0xC7, 0xC0, 0x3C, 0x00, 0x00, 0x00,
        // mov rdi, 0
        0x48, 0xC7, 0xC7, 0x00, 0x00, 0x00, 0x00,
        // syscall
        0x0F, 0x05,
        // ud2
        0x0F, 0x0B,
    ];

    let msg = b"Hello from ELF binary!\n  ";
    assert_eq!(msg.len(), 25);

    // Build minimal ELF64:
    // Offset 0x0000: ELF header (64 bytes)
    // Offset 0x0040: Program header 0 - code segment (PT_LOAD at 0x400000)
    // Offset 0x0080: Program header 1 - data segment (PT_LOAD at 0x401000)
    // Offset 0x1000: Code bytes
    // Offset 0x2000: Message bytes

    let mut elf = { let mut v: Vec<u8> = Vec::new(); v.resize(0x3000, 0u8); v };

    // ELF Header
    elf[0..4].copy_from_slice(&ELF_MAGIC);
    elf[4] = 2;    // EI_CLASS = ELFCLASS64
    elf[5] = 1;    // EI_DATA = ELFDATA2LSB
    elf[6] = 1;    // EI_VERSION
    // bytes 7-15: padding

    write_u16_le(&mut elf, 16, ET_EXEC);     // e_type
    write_u16_le(&mut elf, 18, EM_X86_64);   // e_machine
    write_u32_le(&mut elf, 20, 1);           // e_version
    write_u64_le(&mut elf, 24, 0x401000);    // e_entry (code at 0x401000 since data at 0x400000)
    write_u64_le(&mut elf, 32, 0x40);        // e_phoff = 64 (right after header)
    write_u64_le(&mut elf, 40, 0);           // e_shoff
    write_u32_le(&mut elf, 48, 0);           // e_flags
    write_u16_le(&mut elf, 52, 64);          // e_ehsize
    write_u16_le(&mut elf, 54, 56);          // e_phentsize
    write_u16_le(&mut elf, 56, 2);           // e_phnum = 2 segments
    write_u16_le(&mut elf, 58, 64);          // e_shentsize
    write_u16_le(&mut elf, 60, 0);           // e_shnum
    write_u16_le(&mut elf, 62, 0);           // e_shstrndx

    // Program Header 0: data segment (message at 0x400000)
    let ph0 = 0x40usize;
    write_u32_le(&mut elf, ph0,      PT_LOAD);   // p_type
    write_u32_le(&mut elf, ph0 + 4,  PF_R | PF_W); // p_flags
    write_u64_le(&mut elf, ph0 + 8,  0x2000);    // p_offset (in file)
    write_u64_le(&mut elf, ph0 + 16, 0x400000);  // p_vaddr
    write_u64_le(&mut elf, ph0 + 24, 0x400000);  // p_paddr
    write_u64_le(&mut elf, ph0 + 32, msg.len() as u64); // p_filesz
    write_u64_le(&mut elf, ph0 + 40, 0x1000);    // p_memsz (full page)
    write_u64_le(&mut elf, ph0 + 48, 0x1000);    // p_align

    // Program Header 1: code segment (0x401000)
    let ph1 = 0x40 + 56usize;
    write_u32_le(&mut elf, ph1,      PT_LOAD);       // p_type
    write_u32_le(&mut elf, ph1 + 4,  PF_R | PF_X);  // p_flags
    write_u64_le(&mut elf, ph1 + 8,  0x1000);        // p_offset (in file)
    write_u64_le(&mut elf, ph1 + 16, 0x401000);      // p_vaddr
    write_u64_le(&mut elf, ph1 + 24, 0x401000);      // p_paddr
    write_u64_le(&mut elf, ph1 + 32, code.len() as u64); // p_filesz
    write_u64_le(&mut elf, ph1 + 40, 0x1000);        // p_memsz
    write_u64_le(&mut elf, ph1 + 48, 0x1000);        // p_align

    // Code at file offset 0x1000
    elf[0x1000..0x1000 + code.len()].copy_from_slice(code);

    // Message at file offset 0x2000
    elf[0x2000..0x2000 + msg.len()].copy_from_slice(msg);

    elf
}

// ---------------------------------------------------------------------------
// Little-endian write helpers
// ---------------------------------------------------------------------------

fn write_u16_le(buf: &mut [u8], offset: usize, val: u16) {
    buf[offset]   = (val & 0xff) as u8;
    buf[offset+1] = (val >> 8) as u8;
}

fn write_u32_le(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset]   = (val & 0xff) as u8;
    buf[offset+1] = ((val >> 8)  & 0xff) as u8;
    buf[offset+2] = ((val >> 16) & 0xff) as u8;
    buf[offset+3] = ((val >> 24) & 0xff) as u8;
}

fn write_u64_le(buf: &mut [u8], offset: usize, val: u64) {
    for i in 0..8 {
        buf[offset+i] = ((val >> (i * 8)) & 0xff) as u8;
    }
}
