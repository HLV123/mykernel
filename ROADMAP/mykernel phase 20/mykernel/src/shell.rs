use crate::{print, println};
use alloc::string::String;
use conquer_once::spin::OnceCell;
use core::{pin::Pin, task::{Context, Poll}};
use crossbeam_queue::ArrayQueue;
use futures_util::{stream::Stream, task::AtomicWaker, StreamExt};
use pc_keyboard::{layouts, DecodedKey, HandleControl, Keyboard, ScancodeSet1};

static SCANCODE_QUEUE: OnceCell<ArrayQueue<u8>> = OnceCell::uninit();
static WAKER: AtomicWaker = AtomicWaker::new();

pub(crate) fn add_scancode(scancode: u8) {
    if let Ok(queue) = SCANCODE_QUEUE.try_get() {
        if queue.push(scancode).is_ok() { WAKER.wake(); }
    }
}

struct ScancodeStream { _private: () }
impl ScancodeStream {
    fn new() -> Self {
        SCANCODE_QUEUE.try_init_once(|| ArrayQueue::new(100)).ok();
        ScancodeStream { _private: () }
    }
}
impl Stream for ScancodeStream {
    type Item = u8;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<u8>> {
        let queue = match SCANCODE_QUEUE.try_get() { Ok(q) => q, Err(_) => return Poll::Pending };
        if let Some(sc) = queue.pop() { return Poll::Ready(Some(sc)); }
        WAKER.register(&cx.waker());
        match queue.pop() { Some(sc) => { WAKER.take(); Poll::Ready(Some(sc)) } None => Poll::Pending }
    }
}

static BOOT_TICKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub fn tick() { BOOT_TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed); }
fn uptime_seconds() -> u64 { BOOT_TICKS.load(core::sync::atomic::Ordering::Relaxed) / 18 }

fn dispatch(cmd: &str) {
    let cmd = cmd.trim();
    let (name, args) = match cmd.find(' ') {
        Some(i) => (&cmd[..i], cmd[i+1..].trim()),
        None => (cmd, ""),
    };
    match name {
        "" => {}
        "help" => {
            println!("Commands: help, echo, clear, uptime, mem, ls, cat, write, reboot");
        }
        "echo" => { println!("{}", args); }
        "clear" => { for _ in 0..25 { println!(); } print_prompt(); return; }
        "uptime" => { println!("Uptime: {}s", uptime_seconds()); }
        "mem" => {
            println!("Heap: 0x{:x} ({} KiB)",
                crate::allocator::HEAP_START, crate::allocator::HEAP_SIZE / 1024);
        }
        "ls" => {
            let path = if args.is_empty() { "/" } else { args };
            match crate::fs::readdir(path) {
                Ok(entries) => {
                    println!("{}:", path);
                    for e in entries {
                        let t = match e.file_type {
                            crate::fs::FileType::Directory => 'd',
                            _ => '-',
                        };
                        println!("  {}{:<20} {}", t, e.name, e.size);
                    }
                }
                Err(e) => println!("ls: {:?}", e),
            }
        }
        "cat" => {
            if args.is_empty() { println!("cat: missing filename"); }
            else {
                match crate::fs::open(args, crate::fs::OpenFlags::READ) {
                    Ok(file) => {
                        let mut buf = [0u8; 512];
                        loop {
                            match file.lock().read(&mut buf) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => {
                                    for &b in &buf[..n] {
                                        if b.is_ascii() && (b >= 0x20 || b == b'\n') {
                                            print!("{}", b as char);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => println!("cat: {:?}", e),
                }
            }
        }
        "write" => {
            // write <path> <content>
            match args.find(' ') {
                None => println!("write: usage: write <path> <content>"),
                Some(i) => {
                    let path = &args[..i];
                    let content = &args[i+1..];
                    match crate::fs::create(path) {
                        Ok(file) => {
                            let _ = file.lock().write(content.as_bytes());
                            println!("wrote {} bytes to {}", content.len(), path);
                        }
                        Err(e) => println!("write: {:?}", e),
                    }
                }
            }
        }
        "disk" => { cmd_disk(); }
        "reboot" => {
            println!("Rebooting...");
            unsafe { let ptr = 0xdeadbeefu64 as *mut u8; core::ptr::write_volatile(ptr, 0); }
        }
        _ => { println!("Unknown: '{}'. Type 'help'.", name); }
    }
    print_prompt();
}

fn print_prompt() { print!("\nkernel> "); }

pub async fn run_shell() {
    let mut scancodes = ScancodeStream::new();
    let mut keyboard = Keyboard::new(ScancodeSet1::new(), layouts::Us104Key, HandleControl::Ignore);
    let mut line = String::new();

    println!("=== MyKernel Shell (Phase 14: VFS) ===");
    println!("Commands: help, ls, cat, write, echo, uptime, mem");
    print_prompt();

    while let Some(scancode) = scancodes.next().await {
        if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
            if let Some(key) = keyboard.process_keyevent(key_event) {
                match key {
                    DecodedKey::Unicode('\n') => { println!(); dispatch(&line); line.clear(); }
                    DecodedKey::Unicode('\x08') => { if !line.is_empty() { line.pop(); print!("\x08 \x08"); } }
                    DecodedKey::Unicode(c) => { print!("{}", c); line.push(c); }
                    DecodedKey::RawKey(_) => {}
                }
            }
        }
    }
}

fn cmd_disk() {
    use crate::drivers::{num_sectors, read_sector, SECTOR_SIZE};
    match num_sectors() {
        Some(n) => {
            crate::println!("Disk: {} sectors ({} MiB)", n, n * 512 / 1024 / 1024);
            let mut buf = [0u8; SECTOR_SIZE];
            match read_sector(0, &mut buf) {
                Ok(()) => {
                    crate::print!("Sector 0: ");
                    for i in 0..16 { crate::print!("{:02x} ", buf[i]); }
                    crate::println!("");
                }
                Err(e) => crate::println!("Read error: {}", e),
            }
        }
        None => crate::println!("No disk device"),
    }
}
