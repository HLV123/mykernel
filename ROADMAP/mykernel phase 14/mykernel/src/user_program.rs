use x86_64::{
    structures::paging::{FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB},
    VirtAddr,
};
use crate::process::AddressSpace;

pub const USER_CODE_ADDR: u64 = 0x400000;
pub const USER_STACK_ADDR: u64 = 0x800000;
pub const USER_STACK_SIZE: usize = 4096 * 4;
pub const USER_MSG_ADDR: u64 = 0x401000;

static USER_CODE: &[u8] = &[
    0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00,
    0x48, 0xC7, 0xC7, 0x01, 0x00, 0x00, 0x00,
    0x48, 0xBE, 0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x48, 0xC7, 0xC2, 0x20, 0x00, 0x00, 0x00,
    0x0F, 0x05,
    0x48, 0xC7, 0xC0, 0x3C, 0x00, 0x00, 0x00,
    0x48, 0xC7, 0xC7, 0x00, 0x00, 0x00, 0x00,
    0x0F, 0x05,
    0x0F, 0x0B,
];

pub fn setup_user_memory(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) {
    use PageTableFlags as Flags;
    let code_frame = frame_allocator.allocate_frame().expect("no frame");
    let code_page = Page::containing_address(VirtAddr::new(USER_CODE_ADDR));
    unsafe {
        mapper.map_to(code_page, code_frame, Flags::PRESENT|Flags::WRITABLE|Flags::USER_ACCESSIBLE, frame_allocator).unwrap().flush();
        core::ptr::copy_nonoverlapping(USER_CODE.as_ptr(), USER_CODE_ADDR as *mut u8, USER_CODE.len());
    }
    let msg_frame = frame_allocator.allocate_frame().expect("no frame");
    let msg_page = Page::containing_address(VirtAddr::new(USER_MSG_ADDR));
    unsafe {
        mapper.map_to(msg_page, msg_frame, Flags::PRESENT|Flags::WRITABLE|Flags::USER_ACCESSIBLE, frame_allocator).unwrap().flush();
        core::ptr::write_bytes(USER_MSG_ADDR as *mut u8, 0, 4096);
        let msg = b"Hello from Ring 3!\n              ";
        core::ptr::copy_nonoverlapping(msg.as_ptr(), USER_MSG_ADDR as *mut u8, msg.len().min(32));
    }
    for i in 0..4u64 {
        let sp = Page::containing_address(VirtAddr::new(USER_STACK_ADDR-(i+1)*4096));
        let sf = frame_allocator.allocate_frame().expect("no frame");
        unsafe { mapper.map_to(sp, sf, Flags::PRESENT|Flags::WRITABLE|Flags::USER_ACCESSIBLE, frame_allocator).unwrap().flush(); }
    }
}

pub fn setup_user_memory_in(
    addr_space: &mut AddressSpace,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
    message: &[u8],
) {
    use PageTableFlags as Flags;

    // Allocate fresh frame, write via phys addr, then map into address space
    let code_frame = frame_allocator.allocate_frame().expect("no frame for code");
    let code_phys = phys_mem_offset + code_frame.start_address().as_u64();
    unsafe { core::ptr::copy_nonoverlapping(USER_CODE.as_ptr(), code_phys.as_mut_ptr(), USER_CODE.len()); }
    let code_page = Page::containing_address(VirtAddr::new(USER_CODE_ADDR));
    addr_space.map_page(code_page, code_frame, Flags::PRESENT|Flags::WRITABLE|Flags::USER_ACCESSIBLE, frame_allocator, phys_mem_offset).expect("map code");

    let msg_frame = frame_allocator.allocate_frame().expect("no frame for msg");
    let msg_phys = phys_mem_offset + msg_frame.start_address().as_u64();
    unsafe {
        core::ptr::write_bytes(msg_phys.as_mut_ptr::<u8>(), 0, 4096);
        let len = message.len().min(32);
        core::ptr::copy_nonoverlapping(message.as_ptr(), msg_phys.as_mut_ptr(), len);
    }
    let msg_page = Page::containing_address(VirtAddr::new(USER_MSG_ADDR));
    addr_space.map_page(msg_page, msg_frame, Flags::PRESENT|Flags::WRITABLE|Flags::USER_ACCESSIBLE, frame_allocator, phys_mem_offset).expect("map msg");

    for i in 0..4u64 {
        let stack_frame = frame_allocator.allocate_frame().expect("no frame for stack");
        let stack_page = Page::containing_address(VirtAddr::new(USER_STACK_ADDR-(i+1)*4096));
        addr_space.map_page(stack_page, stack_frame, Flags::PRESENT|Flags::WRITABLE|Flags::USER_ACCESSIBLE, frame_allocator, phys_mem_offset).expect("map stack");
    }
    crate::serial_println!("[user] Process mapped in isolated address space");
}

// Process B dùng L4[1] space để tránh conflict với process A (L4[0])
pub const USER_CODE_ADDR_B: u64 = 0x8000400000;
pub const USER_STACK_ADDR_B: u64 = 0x8000800000;
pub const USER_MSG_ADDR_B: u64 = 0x8000401000;

// Machine code giống USER_CODE nhưng rsi trỏ tới USER_MSG_ADDR_B
pub static USER_CODE_B: &[u8] = &[
    0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00,
    0x48, 0xC7, 0xC7, 0x01, 0x00, 0x00, 0x00,
    // mov rsi, 0x8000401000
    0x48, 0xBE,
    0x00, 0x10, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00,
    0x48, 0xC7, 0xC2, 0x20, 0x00, 0x00, 0x00,
    0x0F, 0x05,
    0x48, 0xC7, 0xC0, 0x3C, 0x00, 0x00, 0x00,
    0x48, 0xC7, 0xC7, 0x00, 0x00, 0x00, 0x00,
    0x0F, 0x05,
    0x0F, 0x0B,
];

pub fn setup_user_memory_b(
    addr_space: &mut AddressSpace,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
    message: &[u8],
) {
    use PageTableFlags as Flags;
    let code_frame = frame_allocator.allocate_frame().expect("no frame");
    let code_phys = phys_mem_offset + code_frame.start_address().as_u64();
    unsafe { core::ptr::copy_nonoverlapping(USER_CODE_B.as_ptr(), code_phys.as_mut_ptr(), USER_CODE_B.len()); }
    addr_space.map_page(Page::containing_address(VirtAddr::new(USER_CODE_ADDR_B)), code_frame,
        Flags::PRESENT|Flags::WRITABLE|Flags::USER_ACCESSIBLE, frame_allocator, phys_mem_offset).expect("map code B");

    let msg_frame = frame_allocator.allocate_frame().expect("no frame");
    let msg_phys = phys_mem_offset + msg_frame.start_address().as_u64();
    unsafe {
        core::ptr::write_bytes(msg_phys.as_mut_ptr::<u8>(), 0, 4096);
        core::ptr::copy_nonoverlapping(message.as_ptr(), msg_phys.as_mut_ptr(), message.len().min(32));
    }
    addr_space.map_page(Page::containing_address(VirtAddr::new(USER_MSG_ADDR_B)), msg_frame,
        Flags::PRESENT|Flags::WRITABLE|Flags::USER_ACCESSIBLE, frame_allocator, phys_mem_offset).expect("map msg B");

    for i in 0..4u64 {
        let sf = frame_allocator.allocate_frame().expect("no frame");
        addr_space.map_page(Page::containing_address(VirtAddr::new(USER_STACK_ADDR_B-(i+1)*4096)), sf,
            Flags::PRESENT|Flags::WRITABLE|Flags::USER_ACCESSIBLE, frame_allocator, phys_mem_offset).expect("map stack B");
    }
    crate::serial_println!("[user] Process B mapped at L4[1] space");
}
