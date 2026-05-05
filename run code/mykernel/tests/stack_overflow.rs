#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

use core::panic::PanicInfo;
use mykernel::{exit_qemu, serial_println, QemuExitCode};
use lazy_static::lazy_static;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

/// Test này verify double fault handler chạy được khi stack overflow
/// Double fault handler phải dùng IST stack riêng, không dùng kernel stack bị overflow

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[stack_overflow] Starting test");

    mykernel::gdt::init();
    init_test_idt();

    // Trigger stack overflow bằng infinite recursion
    stack_overflow();

    panic!("Execution continued after stack overflow — should not reach here");
}

#[allow(unconditional_recursion)]
fn stack_overflow() {
    stack_overflow(); // Đệ quy vô tận → stack overflow → double fault
    volatile::Volatile::new(0).read(); // Ngăn tail-call optimization
}

lazy_static! {
    static ref TEST_IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        unsafe {
            idt.double_fault
                .set_handler_fn(test_double_fault_handler)
                .set_stack_index(mykernel::gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt
    };
}

fn init_test_idt() {
    TEST_IDT.load();
}

extern "x86-interrupt" fn test_double_fault_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    serial_println!("[ok] Double fault handler triggered correctly");
    exit_qemu(QemuExitCode::Success);
    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    mykernel::test_panic_handler(info)
}
