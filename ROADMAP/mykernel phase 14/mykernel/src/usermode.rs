use crate::{print, println, serial_println};
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, SFMask};
use x86_64::registers::rflags::RFlags;
use x86_64::VirtAddr;

pub const SYS_WRITE: u64 = 1;
pub const SYS_EXIT:  u64 = 60;
pub const SYS_GETPID: u64 = 39;

pub fn init_syscalls() {
    unsafe {
        Efer::update(|f| *f |= EferFlags::SYSTEM_CALL_EXTENSIONS);
        LStar::write(VirtAddr::new(syscall_handler as *const () as u64));
        let star_val: u64 = (0x0010u64 << 48) | (0x0008u64 << 32);
        x86_64::registers::model_specific::Msr::new(0xC000_0081).write(star_val);
        SFMask::write(RFlags::INTERRUPT_FLAG);
    }
    init_kernel_syscall_stack();
    serial_println!("[syscall] initialized, handler={:#x}",
        syscall_handler as *const () as u64);
}

#[unsafe(naked)]
pub unsafe extern "C" fn syscall_handler() {
    core::arch::naked_asm!(
        "mov r10, rsp",
        "mov rsp, [{kstack}]",
        "push r10",
        "push rcx",
        "push r11",
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov rcx, rdx",
        "mov rdx, rsi",
        "mov rsi, rdi",
        "mov rdi, rax",
        "call {handler}",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "pop r11",
        "pop rcx",
        "pop r10",
        "mov rsp, r10",
        "sysretq",
        kstack = sym KERNEL_SYSCALL_STACK_TOP,
        handler = sym handle_syscall,
    );
}

static mut SYSCALL_STACK: [u8; 4096 * 4] = [0u8; 4096 * 4];
static mut KERNEL_SYSCALL_STACK_TOP: u64 = 0;

pub fn init_kernel_syscall_stack() {
    unsafe {
        let stack_start = core::ptr::addr_of!(SYSCALL_STACK) as u64;
        KERNEL_SYSCALL_STACK_TOP = stack_start + (4096 * 4) as u64;
        serial_println!("[syscall] kernel stack top = {:#x}", KERNEL_SYSCALL_STACK_TOP);
    }
}

#[no_mangle]
extern "C" fn handle_syscall(syscall_nr: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    match syscall_nr {
        SYS_WRITE  => sys_write(arg1, arg2 as *const u8, arg3),
        SYS_EXIT   => sys_exit(arg1),
        SYS_GETPID => 42,
        _ => { serial_println!("[syscall] unknown: {}", syscall_nr); u64::MAX }
    }
}

fn sys_write(fd: u64, buf_ptr: *const u8, len: u64) -> u64 {
    if fd != 1 && fd != 2 { return u64::MAX; }
    if buf_ptr as u64 >= 0x8000_0000_0000 { return u64::MAX; }
    let bytes = unsafe { core::slice::from_raw_parts(buf_ptr, len as usize) };
    for &byte in bytes {
        if byte.is_ascii() && (byte >= 0x20 || byte == b'\n') {
            print!("{}", byte as char);
        }
    }
    serial_println!("[sys_write] fd={} len={}", fd, len);
    len
}

fn sys_exit(exit_code: u64) -> u64 {
    println!("[user] Process exited with code {}", exit_code);
    serial_println!("[syscall] sys_exit({})", exit_code);
    loop { x86_64::instructions::hlt(); }
}

pub fn enter_user_mode_with_stack(entry: u64, user_stack_top: u64) -> ! {
    let user_cs = crate::gdt::GDT.1.user_code.0 as u64;
    let user_ss = crate::gdt::GDT.1.user_data.0 as u64;
    serial_println!("[usermode] entry={:#x} stack={:#x} cs={:#x} ss={:#x}",
        entry, user_stack_top, user_cs, user_ss);
    unsafe {
        core::arch::asm!(
            "push {ss}",
            "push {rsp}",
            "pushf",
            "or qword ptr [rsp], 0x200",
            "push {cs}",
            "push {rip}",
            "iretq",
            ss  = in(reg) user_ss,
            rsp = in(reg) user_stack_top,
            cs  = in(reg) user_cs,
            rip = in(reg) entry,
            options(noreturn),
        );
    }
}
