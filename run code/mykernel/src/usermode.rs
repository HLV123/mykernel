use crate::{print, println, serial_println};
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, SFMask};
use x86_64::registers::rflags::RFlags;
use x86_64::VirtAddr;

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

/// Syscall handler — Linux x86_64 ABI:
///   rax = syscall number
///   rdi = arg1, rsi = arg2, rdx = arg3
///   r10 = arg4, r8 = arg5, r9 = arg6
///   rcx = user RIP (saved by syscall), r11 = user RFLAGS
#[unsafe(naked)]
pub unsafe extern "C" fn syscall_handler() {
    core::arch::naked_asm!(
        // Save user RSP, switch to kernel stack
        "mov r10, rsp",
        "mov rsp, [{kstack}]",
        // Save context
        "push r10",   // user RSP
        "push rcx",   // user RIP
        "push r11",   // user RFLAGS
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        // Linux syscall ABI → C calling convention:
        // rdi=arg1, rsi=arg2, rdx=arg3, r10=arg4, r8=arg5, r9=arg6
        // dispatch(nr, a1, a2, a3, a4, a5, a6)
        // rdi=nr, rsi=a1, rdx=a2, rcx=a3, r8=a4, r9=a5, push a6
        "push r9",     // a6 — push before shifting
        "mov r9,  r8", // a5
        "mov r8,  r10",// a4 (r10 was saved, but we need original r10 = arg4)
        "mov rcx, rdx",// a3
        "mov rdx, rsi",// a2
        "mov rsi, rdi",// a1
        "mov rdi, rax",// nr
        "call {handler}",
        "add rsp, 8",  // pop a6
        // rax = return value from dispatch()
        // Restore
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
        handler = sym crate::syscall::dispatch,
    );
}

static mut SYSCALL_STACK: [u8; 4096 * 8] = [0u8; 4096 * 8];
static mut KERNEL_SYSCALL_STACK_TOP: u64 = 0;

pub fn init_kernel_syscall_stack() {
    unsafe {
        let stack_start = core::ptr::addr_of!(SYSCALL_STACK) as u64;
        KERNEL_SYSCALL_STACK_TOP = stack_start + (4096 * 8) as u64;
        serial_println!("[syscall] kernel stack top = {:#x}", KERNEL_SYSCALL_STACK_TOP);
    }
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
