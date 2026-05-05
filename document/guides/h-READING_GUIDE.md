# Hướng Dẫn Đọc Code

> Hướng dẫn đọc source code của MyKernel theo thứ tự hợp lý — từ file nào trước, file nào sau, và những đoạn code quan trọng cần chú ý.

---

## Nguyên tắc chung

Đọc theo thứ tự **bottom-up**: hardware → kernel core → subsystems → shell. Đừng đọc `shell.rs` trước khi hiểu `interrupts.rs` — bạn sẽ thấy code như magic.

Mỗi lần đọc 1 file, hãy tự hỏi:
1. File này làm gì? (đọc comment đầu file)
2. Public functions/types quan trọng nhất là gì?
3. File này gọi đến file nào khác?
4. Ai gọi đến file này?

---

## Thứ tự đọc được khuyến nghị

### Bước 1: Entry Point

**File: `src/main.rs`**

Đọc toàn bộ — chỉ ~120 dòng. Đây là nơi kernel bắt đầu. Mỗi dòng `mykernel::xxx::init()` tương ứng với 1 subsystem.

Câu hỏi cần trả lời:
- Thứ tự init các subsystem là gì?
- Tại sao GDT phải init trước IDT?
- Tại sao heap phải init trước filesystem?

```rust
fn kernel_main(boot_info: &'static BootInfo) -> ! {
    mykernel::init();           // GDT + IDT + PIC
    // memory init...
    mykernel::allocator::init_heap(...);   // heap phải trước fs
    mykernel::fs::init();       // fs cần heap để allocate
    mykernel::drivers::init();  // drivers cần fs và heap
    mykernel::net::init();      // net cần drivers
    mykernel::security::init(); // security sau cùng
    // ...
}
```

---

### Bước 2: CPU Setup

**File: `src/gdt.rs`**

Đọc để hiểu GDT có gì. Chú ý:
- 5 segment descriptors (null, kernel code, kernel data, user code, user data)
- TSS entry và tại sao cần

**File: `src/interrupts.rs`**

Đây là file quan trọng. Đọc theo thứ tự:
1. `init_idt()` — xem IDT được setup thế nào
2. `breakpoint_handler` — handler đơn giản nhất
3. `double_fault_handler` — xem IST được dùng thế nào
4. `timer_handler` — tick counter, EOI
5. `keyboard_handler` — đọc scancode, push vào queue

Đoạn code quan trọng:
```rust
// keyboard handler
extern "x86-interrupt" fn keyboard_handler(_stack_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;
    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };
    crate::task::keyboard::add_scancode(scancode);  // push vào queue
    unsafe { crate::apic::end_of_interrupt(); }     // báo APIC xong
}
```

---

### Bước 3: Memory

**File: `src/memory.rs`**

Chú ý 2 phần:
1. `init()` — tạo mapper từ boot_info
2. `BootInfoFrameAllocator` — iterate memory map để lấy frames

**File: `src/allocator.rs`**

Ngắn, dễ đọc. Chú ý:
- `HEAP_START` và `HEAP_SIZE` constants
- `init_heap()` map virtual pages → physical frames

---

### Bước 4: Async Foundation

**File: `src/task/keyboard.rs`**

Đọc để hiểu async keyboard input:
1. `SCANCODE_QUEUE` — lock-free queue từ crossbeam
2. `add_scancode()` — called từ interrupt handler, push vào queue + wake
3. `ScancodeStream` — implement `futures::Stream`
4. `read_key()` — await từ shell

**File: `src/task/executor.rs`**

Đọc để hiểu async executor:
1. `Task` struct — wrapper quanh Future
2. `Executor::run()` — poll loop, HLT khi không có task

Đoạn code quan trọng:
```rust
fn run(&mut self) -> ! {
    loop {
        self.run_ready_tasks();  // poll tất cả ready tasks
        self.sleep_if_idle();    // HLT nếu không có task nào ready
    }
}

fn sleep_if_idle(&self) {
    if self.task_queue.is_empty() {
        x86_64::instructions::interrupts::enable_and_hlt();
        // Interrupt wakes CPU → loop lại
    }
}
```

---

### Bước 5: Filesystem

**File: `src/fs/vfs.rs`**

Đọc theo thứ tự:
1. `FileSystem` trait — interface mà mọi FS implement
2. `File` trait — interface cho file handles
3. `MOUNT_TABLE` — global mount table
4. `mount()`, `open()`, `readdir()` — VFS functions

**File: `src/fs/ramfs.rs`**

Xem RamFS implement FileSystem trait thế nào:
1. `INode` struct — data + file_type
2. `normalize()` — chuẩn hóa path
3. `impl FileSystem for RamFs` — mỗi function làm gì

**File: `src/fs/initramfs.rs`**

Xem CPIO parser:
1. `parse_entry()` — parse 1 CPIO entry
2. `load_into_ramfs()` — unpack archive vào VFS

---

### Bước 6: Shell

**File: `src/shell.rs`**

Bây giờ bạn đã hiểu VFS và async, đọc shell:
1. `run_shell()` — async loop: read line → dispatch
2. `try_read_serial()` — poll UART COM1
3. `read_line()` — đọc từng ký tự, handle backspace
4. Mỗi `cmd_*` function — implementation của 1 lệnh

Chú ý `cmd_ls`:
```rust
fn cmd_ls(path: &str) {
    match crate::fs::readdir(path) {
        Ok(entries) => {
            for entry in &entries {
                let t = match entry.file_type {
                    FileType::Directory   => 'd',
                    FileType::RegularFile => '-',
                    _                     => '?',
                };
                println!("  {}  {:>8}  {}", t, entry.size, entry.name);
            }
        }
        Err(e) => println!("ls: {}: {:?}", path, e),
    }
}
```

Rất đơn giản — gọi `fs::readdir()` rồi format output.

---

### Bước 7: Network

**File: `src/drivers/virtio_net.rs`**

Đọc để hiểu driver tầng thấp:
1. `init()` — setup virtqueues, đọc MAC
2. `send_packet()` — ghi vào TX queue, notify QEMU
3. `recv_packet()` — poll RX used ring

**File: `src/net/mod.rs`**

Đọc `rx_dispatch()` — phân phối packet theo protocol:
```rust
pub fn rx_dispatch(frame: &[u8]) {
    let ethertype = /* đọc bytes 12-13 */;
    match ethertype {
        0x0806 => process_arp(frame),
        0x0800 => process_ipv4(frame),
        _ => {}
    }
}
```

**File: `src/net/tcp.rs`**

Đọc state machine — phần phức tạp nhất:
1. `TcpState` enum — các trạng thái
2. `process_tcp()` — dispatch theo state
3. `handle_listen()` — khi nhận SYN → gửi SYN-ACK

---

### Bước 8: Security

**File: `src/security.rs`**

Đọc để hiểu các cơ chế bảo mật:
1. `init()` — thứ tự init
2. `init_stack_canary()` — RDTSC + XOR
3. `validate_user_ptr()` — kiểm tra pointer từ user
4. `Capabilities` struct — capability bitmask
5. `SecurityAudit.score()` — tính điểm bảo mật

---

### Bước 9: SMP (đọc sau cùng — phức tạp nhất)

**File: `src/smp.rs`**

Đọc từng phần:
1. `CPU_TABLE` và `CpuInfo` — dữ liệu per-CPU
2. `parse_acpi_madt()` — tìm processors từ ACPI
3. `boot_aps()` — gửi INIT + SIPI IPI

**File: `src/sync.rs`**

Đọc `SpinLock` để hiểu locking:
```rust
pub fn lock(&self) -> SpinLockGuard<T> {
    // Disable interrupts (không deadlock với interrupt handler)
    x86_64::instructions::interrupts::disable();
    // Spin cho đến khi lấy được lock
    while self.locked.compare_exchange(false, true, ...).is_err() {
        core::hint::spin_loop();
    }
    SpinLockGuard { lock: self }
}
// Guard tự động unlock và re-enable interrupts khi drop
```

---

## Những Đoạn Code Khó Hiểu Thường Gặp

### 1. `let x = match file.lock().method() {...}; x`

```rust
// Pattern này có vẻ lạ:
let x = match file.lock().read(buf) {
    Ok(n) => n as i64,
    Err(_) => -1,
}; x
```

**Lý do:** Borrow checker issue. `file.lock()` trả về `MutexGuard` — nếu dùng trực tiếp trong `match`, guard sống đến hết `match` expression, nhưng compiler không thể prove điều đó. Lưu kết quả vào `x` trước cho guard drop, sau đó return `x`.

### 2. `#[unsafe(naked)]` cho context switch

```rust
#[unsafe(naked)]
unsafe extern "C" fn switch_context(old: *mut Context, new: *const Context) {
    core::arch::asm!(
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "ret",
        options(noreturn)
    );
}
```

**Lý do:** Naked function không có Rust-generated prologue/epilogue (không có `push rbp`, `mov rbp, rsp`). Cần để lưu/restore registers hoàn toàn thủ công khi context switch.

### 3. `Arc<Mutex<dyn File>>`

```rust
type FileHandle = Arc<Mutex<dyn File>>;
```

- `Arc`: reference counting — nhiều FdTable entries có thể share 1 file handle
- `Mutex`: thread safety — nhiều threads/CPUs có thể access
- `dyn File`: dynamic dispatch — RamFile, DevFile, Fat32File đều dùng chung interface

### 4. `extern "x86-interrupt" fn handler`

```rust
extern "x86-interrupt" fn breakpoint_handler(frame: InterruptStackFrame) {
    // ...
}
```

**Lý do:** ABI đặc biệt cho interrupt handlers. Rust tự động generate `IRETQ` thay vì `RET` ở cuối, và handle việc save/restore registers theo calling convention của CPU interrupt.

### 5. `unsafe { core::arch::asm!(...) }`

```rust
unsafe {
    core::arch::asm!(
        "in al, dx",
        in("dx") port,
        out("al") value,
        options(nomem, nostack)
    );
}
```

**Lý do:** Inline assembly cho phép dùng CPU instructions mà Rust không expose — port I/O (`in`/`out`), CPU control (`hlt`, `sti`, `cli`), system registers (MSRs, CR0, CR3, CR4).

---

## Tips Khi Đọc Code

1. **Đọc comment trước** — mỗi function và module có comment giải thích mục đích.

2. **Theo dõi data flow** — trace 1 lệnh shell từ đầu đến cuối: `ls /` → `readdir` → VFS → RamFS → response.

3. **Dùng grep** để tìm nơi function được gọi:
   ```bash
   grep -r "add_scancode" src/
   ```

4. **Đọc tests** — `tests/` và các `#[test_case]` trong `main.rs` cho thấy expected behavior.

5. **Chạy với serial output** — thêm `serial_println!` vào bất kỳ chỗ nào để debug:
   ```rust
   crate::serial_println!("[debug] reached here: {}", value);
   ```

6. **Không cần hiểu hết ngay** — đọc top-level rồi drill down khi cần. Không ai nhớ hết 50 files từ đầu.
