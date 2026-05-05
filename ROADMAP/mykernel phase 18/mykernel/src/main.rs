#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![feature(naked_functions)]
#![test_runner(mykernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use mykernel::{print, println, serial_println};
use mykernel::memory::BootInfoFrameAllocator;
use mykernel::task::{executor::Executor, Task};
use mykernel::process::{AddressSpace, Process};
use x86_64::{registers::control::Cr3, VirtAddr};

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Booting MyKernel...");
    println!("Phase 18: Syscalls for musl libc");

    mykernel::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { mykernel::memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe {
        BootInfoFrameAllocator::init(&boot_info.memory_map)
    };

    mykernel::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap init failed");

    mykernel::fs::init();
    mykernel::process::set_phys_mem_offset(boot_info.physical_memory_offset);
    mykernel::usermode::init_syscalls();

    println!("[ok] Kernel initialized");
    println!("[ok] Syscall table: {} syscalls", count_syscalls());

    #[cfg(test)]
    {
        test_main();
        mykernel::exit_qemu(mykernel::QemuExitCode::Success);
        mykernel::hlt_loop()
    }

    #[cfg(not(test))]
    {
        demo_syscalls();

        println!("");
        println!("Phase 18 complete! Syscall table ready for musl libc.");
        println!("Shell starting...");

        let mut executor = Executor::new();
        executor.spawn(Task::new(mykernel::shell::run_shell()));
        executor.run()
    }
}

fn count_syscalls() -> usize {
    // Count implemented syscalls
    use mykernel::syscall::*;
    let syscalls = [
        SYS_READ, SYS_WRITE, SYS_OPEN, SYS_CLOSE,
        SYS_STAT, SYS_FSTAT, SYS_LSTAT, SYS_LSEEK,
        SYS_MMAP, SYS_MPROTECT, SYS_MUNMAP, SYS_BRK,
        SYS_RT_SIGACTION, SYS_RT_SIGPROCMASK, SYS_IOCTL,
        SYS_ACCESS, SYS_NANOSLEEP, SYS_GETPID, SYS_GETPPID,
        SYS_FORK, SYS_EXIT, SYS_EXIT_GROUP, SYS_UNAME,
        SYS_FCNTL, SYS_GETCWD, SYS_CHDIR, SYS_READLINK,
        SYS_GETTIMEOFDAY, SYS_ARCH_PRCTL, SYS_FUTEX,
        SYS_GETDENTS64, SYS_SET_TID_ADDRESS, SYS_CLOCK_GETTIME,
        SYS_SET_ROBUST_LIST, SYS_PRLIMIT64, SYS_GETRANDOM,
        SYS_GETUID, SYS_GETGID, SYS_GETEUID, SYS_GETEGID,
    ];
    syscalls.len()
}

fn demo_syscalls() {
    use mykernel::syscall::*;

    println!("");
    println!("=== Syscall Table Demo ===");

    // Test syscall dispatch directly (kernel-side)
    println!("Testing syscall dispatch...");

    // write to stdout (fd=1)
    let msg = b"[syscall demo] write() to stdout works!\n";
    let ret = dispatch(SYS_WRITE, 1, msg.as_ptr() as u64, msg.len() as u64, 0, 0, 0);
    println!("sys_write returned: {}", ret);

    // getpid
    let pid = dispatch(SYS_GETPID, 0, 0, 0, 0, 0, 0);
    println!("sys_getpid returned: {}", pid);

    // uname
    let mut uname = [0u8; core::mem::size_of::<Utsname>()];
    dispatch(SYS_UNAME, uname.as_mut_ptr() as u64, 0, 0, 0, 0, 0);
    let sysname = core::str::from_utf8(&uname[..5]).unwrap_or("?");
    println!("sys_uname sysname: {}", sysname);

    // brk
    let brk0 = dispatch(SYS_BRK, 0, 0, 0, 0, 0, 0);
    println!("sys_brk(0) = {:#x}", brk0 as u64);

    // open + read + close a file
    let path = b"/etc/hostname\0";
    let fd = dispatch(SYS_OPEN, path.as_ptr() as u64, O_RDONLY, 0, 0, 0, 0);
    println!("sys_open(/etc/hostname) = fd {}", fd);
    if fd >= 0 {
        let mut buf = [0u8; 64];
        let n = dispatch(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, 63, 0, 0, 0);
        if n > 0 {
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("?").trim();
            println!("sys_read: \"{}\" ({} bytes)", s, n);
        }
        dispatch(SYS_CLOSE, fd as u64, 0, 0, 0, 0, 0);
    }

    // getrandom
    let mut rand_buf = [0u8; 8];
    dispatch(SYS_GETRANDOM, rand_buf.as_mut_ptr() as u64, 8, 0, 0, 0, 0);
    print!("sys_getrandom: ");
    for b in &rand_buf { print!("{:02x}", b); }
    println!("");

    // arch_prctl ARCH_SET_FS (needed by musl TLS)
    let fake_tcb = [0u64; 8];
    let ret = dispatch(SYS_ARCH_PRCTL, ARCH_SET_FS, fake_tcb.as_ptr() as u64, 0, 0, 0, 0);
    println!("sys_arch_prctl(ARCH_SET_FS) = {}", ret);

    println!("=== Syscall Table Ready ===");
    println!("");

    serial_println!("[phase18] Syscall demo complete");
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("[PANIC] {}", info);
    serial_println!("[PANIC] {}", info);
    mykernel::hlt_loop()
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! { mykernel::test_panic_handler(info) }

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test_case]
fn test_breakpoint_exception() { x86_64::instructions::interrupts::int3(); }

#[test_case]
fn test_syscall_write() {
    mykernel::fs::init();
    use mykernel::syscall::*;
    let msg = b"test write\n";
    let ret = dispatch(SYS_WRITE, 1, msg.as_ptr() as u64, msg.len() as u64, 0, 0, 0);
    assert_eq!(ret, msg.len() as i64);
    serial_println!("[test] sys_write OK");
}

#[test_case]
fn test_syscall_open_read() {
    mykernel::fs::init();
    use mykernel::syscall::*;
    let path = b"/etc/hostname\0";
    let fd = dispatch(SYS_OPEN, path.as_ptr() as u64, O_RDONLY, 0, 0, 0, 0);
    assert!(fd >= 3, "fd should be >= 3, got {}", fd);

    let mut buf = [0u8; 64];
    let n = dispatch(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, 63, 0, 0, 0);
    assert!(n > 0, "read returned {}", n);

    let ret = dispatch(SYS_CLOSE, fd as u64, 0, 0, 0, 0, 0);
    assert_eq!(ret, 0);
    serial_println!("[test] sys_open+read+close OK: {} bytes", n);
}

#[test_case]
fn test_syscall_stat() {
    mykernel::fs::init();
    use mykernel::syscall::*;
    let path = b"/etc/motd\0";
    let mut stat_buf = [0u8; core::mem::size_of::<LinuxStat>()];
    let ret = dispatch(SYS_STAT, path.as_ptr() as u64, stat_buf.as_mut_ptr() as u64, 0, 0, 0, 0);
    assert_eq!(ret, 0, "stat returned {}", ret);
    let stat = unsafe { &*(stat_buf.as_ptr() as *const LinuxStat) };
    assert!(stat.st_size > 0, "st_size should be > 0");
    serial_println!("[test] sys_stat OK: st_size={}", stat.st_size);
}

#[test_case]
fn test_syscall_uname() {
    use mykernel::syscall::*;
    let mut uname_buf = [0u8; core::mem::size_of::<Utsname>()];
    let ret = dispatch(SYS_UNAME, uname_buf.as_mut_ptr() as u64, 0, 0, 0, 0, 0);
    assert_eq!(ret, 0);
    // Check sysname starts with "Linux"
    assert_eq!(&uname_buf[..5], b"Linux");
    serial_println!("[test] sys_uname OK");
}
