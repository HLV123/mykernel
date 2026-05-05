# Debugging MyKernel

> Hướng dẫn debug kernel — từ đọc serial log đến dùng QEMU GDB stub và phân tích crash.

---

## Serial Log — Công cụ chính

Mọi `serial_println!` output đều xuất hiện trong terminal khi chạy với `-serial stdio`.

```powershell
cargo bootimage
qemu-system-x86_64 `
  -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
  -serial stdio -no-reboot 2>&1 | Tee-Object -FilePath kernel.log
```

`Tee-Object` vừa hiển thị vừa lưu vào file để phân tích sau.

### Thêm debug log vào code

```rust
// Tạm thời thêm vào bất kỳ chỗ nào:
crate::serial_println!("[DEBUG] func={} val={:#x}", function_name, val);

// Trong interrupt handler (cẩn thận — có thể ảnh hưởng timing):
crate::serial_println!("[IRQ] timer tick={}", TICK_COUNT.load(Ordering::Relaxed));
```

### Log markers để trace execution path

```rust
fn complex_function(x: u64) -> Result<u64, Error> {
    crate::serial_println!("[TRACE] complex_function entry x={}", x);

    let step1 = do_step1(x);
    crate::serial_println!("[TRACE] step1={:?}", step1);

    let step2 = do_step2(step1?);
    crate::serial_println!("[TRACE] step2={:?}", step2);

    crate::serial_println!("[TRACE] complex_function exit ok");
    Ok(step2)
}
```

---

## QEMU GDB Stub

QEMU có built-in GDB server. Khi kernel crash hoặc hang, GDB cho phép inspect state.

### Setup

**Terminal 1 — Start QEMU với GDB stub:**
```powershell
qemu-system-x86_64 `
  -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
  -serial stdio -no-reboot `
  -s -S
  # -s = enable GDB server trên port 1234
  # -S = pause CPU tại start (đợi GDB connect)
```

**Terminal 2 — Connect GDB (trên Linux/macOS, hoặc WSL):**
```bash
# Cần debug symbols từ ELF binary
rust-gdb target/x86_64-mykernel/debug/mykernel

# Trong GDB:
(gdb) target remote :1234
(gdb) break kernel_main
(gdb) continue
```

Trên Windows, dùng WSL:
```bash
# Trong WSL:
/mnt/c/.../target/x86_64-mykernel/debug/mykernel
```

### Useful GDB Commands

```bash
# Execution control
(gdb) continue          # tiếp tục chạy
(gdb) stepi             # chạy 1 machine instruction
(gdb) nexti             # chạy 1 instruction, không step vào function
(gdb) finish            # chạy đến return của function hiện tại

# Breakpoints
(gdb) break kernel_main          # break tại function
(gdb) break src/shell.rs:45      # break tại dòng cụ thể
(gdb) watch TICK_COUNT           # break khi biến thay đổi
(gdb) info breakpoints           # list all breakpoints
(gdb) delete 1                   # xóa breakpoint 1

# Inspect state
(gdb) info registers             # tất cả registers
(gdb) print $rax                 # in giá trị register
(gdb) print $rsp                 # stack pointer
(gdb) x/10gx $rsp                # dump 10 quadwords từ stack
(gdb) x/10i $rip                 # disassemble 10 instructions từ RIP
(gdb) print variable_name        # in giá trị biến Rust
(gdb) info locals                # tất cả local variables

# Memory
(gdb) x/1gx 0xfee00000          # đọc LAPIC base
(gdb) x/16bx 0xb8000            # đọc VGA buffer
(gdb) x/10i 0x211e10            # disassemble tại địa chỉ

# Stack trace
(gdb) backtrace                  # call stack
(gdb) frame 2                    # chuyển đến frame 2
```

### Example: Debug Page Fault

```bash
# Kernel crash với page fault tại 0xdeadbeef
(gdb) target remote :1234
(gdb) continue

# Sau khi crash:
(gdb) info registers
rax            0x0                 0
rbx            0x20b000            2142208
rip            0x20fa68            0x20fa68 <some_function+24>
rsp            0x10000200000       0x10000200000

(gdb) backtrace
#0  some_function () at src/memory.rs:145
#1  init_paging () at src/memory.rs:89
#2  kernel_main () at src/main.rs:34

(gdb) frame 0
(gdb) info locals
mapper = { ... }
addr = 0xdeadbeef    ← đây là vấn đề!

(gdb) x/10i $rip-10  # disassemble quanh crash point
```

---

## QEMU Monitor

QEMU có interactive monitor. Nhấn **Ctrl+A C** (khi dùng `-nographic` hoặc `-serial stdio`) để vào monitor.

```
QEMU> help                    # list tất cả commands
QEMU> info registers          # CPU registers
QEMU> info mem                # virtual memory mappings
QEMU> info tlb                # TLB entries
QEMU> info cpus               # CPU states
QEMU> info pci                # PCI devices
QEMU> x /10gx 0xfee00000      # memory dump
QEMU> xp /10gx 0x1000000      # physical memory dump
QEMU> p $rip                  # print register
QEMU> gdbserver tcp::1235     # start GDB server trên port khác
QEMU> quit                    # thoát
```

### Đặc biệt hữu ích:

```
QEMU> info mem
0000000000000000-0000000000001000 0000000000001000 -rw
...
0000004444440000-0000004444540000 0000000000100000 -rw   ← heap
00000180fee00000-00000180fee01000 0000000000001000 -rw   ← LAPIC MMIO

QEMU> info tlb
00000000b8000: 00000000000b8000 ----A-
0000004444440000: 0000000002000000 ---DA-   ← heap physical addr
```

---

## Đọc Kernel Panic Messages

```
--- KERNEL PANIC ---
panicked at 'memory allocation of 102400 bytes failed',
C:\...\alloc\src\alloc.rs:573:9

KERNEL PANIC: panicked at 'memory allocation of 102400 bytes failed',
C:\...\alloc\src\alloc.rs:573:9
```

**Phân tích:**
- Allocation 102400 bytes = 100KB → đây là virtio-net RX buffer allocation
- `alloc.rs:573` = `handle_alloc_error` trong standard allocator
- Fix: tăng `HEAP_SIZE` trong `allocator.rs`

---

## Common Panic Messages và Nguyên Nhân

### `memory allocation of X bytes failed`

```
Nguyên nhân: HEAP_SIZE quá nhỏ
Fix: src/allocator.rs → tăng HEAP_SIZE
     pub const HEAP_SIZE: usize = 512 * 1024; // 512KB
```

### `attempt to subtract with overflow`

```
Nguyên nhân: integer underflow trong release mode (debug mode catches this)
Fix: dùng checked_sub() hoặc saturating_sub()
     let result = a.checked_sub(b).unwrap_or(0);
```

### `attempt to index out of bounds`

```
Nguyên nhân: array index >= array length
Fix: thêm bounds check, hoặc dùng .get(i) thay vì [i]
     if let Some(val) = array.get(i) { ... }
```

### `unwrap() on a None value`

```
Nguyên nhân: Option::unwrap() khi value = None
Stack trace cho biết dòng nào
Fix: xử lý None case, hoặc thêm expect("message") để rõ nguyên nhân
```

### `ScancodeStream::new should only be called once`

```
Nguyên nhân: SCANCODE_QUEUE được init 2 lần
Fix: chỉ tạo ScancodeStream một lần duy nhất trong executor
```

### Triple fault / Machine reset (không có output)

```
Nguyên nhân thường gặp:
1. Stack overflow trước khi IST setup → triple fault
2. Page fault trong fault handler (không có IST)
3. Invalid GDT entry

Debug: chạy QEMU với -d int,cpu_reset để xem interrupt log
qemu-system-x86_64 -d int,cpu_reset -D qemu.log ...
```

---

## QEMU Debug Flags

```powershell
# Log tất cả interrupts và CPU resets
-d int,cpu_reset -D qemu.log

# Log page faults
-d page -D qemu.log

# Log tất cả guest instructions (rất chậm, verbose)
-d in_asm -D qemu.log

# Log guest memory access
-d nochain,in_asm -D qemu.log
```

**Đọc qemu.log:**
```
----------------
IN:
0x00020fa0:  push   rbp
0x00020fa1:  mov    rbp,rsp

...

check_exception old: 0xffffffff new 0xe   ← exception 0xe = page fault!
    14: v=0e e=0002 i=0 cpl=0 IP=0008:00020fb4 pc=00020fb4 SP=0010:00000000102008f0
    CR2=00000000deadbeef               ← faulting address!
```

---

## Phân Tích Core Dump (Linux host)

Khi chạy QEMU trên Linux, có thể dump memory state:

```bash
# Trong QEMU monitor:
QEMU> dump-guest-memory /tmp/kernel.dump

# Phân tích với crash tool hoặc GDB:
gdb target/x86_64-mykernel/debug/mykernel /tmp/kernel.dump
```

---

## Thêm Assert để Catch Bugs Sớm

```rust
// Trong development, thêm assertions:
fn allocate_frame(&mut self) -> Option<PhysFrame> {
    debug_assert!(
        self.next < self.usable_frames.clone().count(),
        "frame allocator out of bounds"
    );
    // ...
}

// Assert invariants tại entry point:
pub fn mount(path: &str, fs: Arc<dyn FileSystem>) {
    assert!(!path.is_empty(), "mount path cannot be empty");
    assert!(path.starts_with('/'), "mount path must be absolute");
    // ...
}
```

`debug_assert!` chỉ compile trong debug build — không có overhead trong release.

---

## Debugging SMP Issues

### Race conditions khó reproduce

```rust
// Thêm delay để reproduce race condition:
fn problematic_function() {
    let guard = SHARED_STATE.lock();
    
    // Thêm delay để CPU khác có thể interleave:
    #[cfg(debug_assertions)]
    for _ in 0..1000 { core::hint::spin_loop(); }
    
    // Critical section
    *guard += 1;
}
```

### Kiểm tra APIC ID

```rust
// Verify code chạy trên đúng CPU:
fn must_run_on_bsp() {
    let id = crate::apic::lapic_id();
    assert_eq!(id, 0, "this must run on BSP (APIC ID 0), got {}", id);
}
```

### Log per-CPU events

```rust
fn per_cpu_init() {
    let cpu_id = crate::apic::lapic_id();
    crate::serial_println!("[CPU {}] per-cpu init start", cpu_id);
    // ...
    crate::serial_println!("[CPU {}] per-cpu init done", cpu_id);
}
```

---

## Checklist Khi Debug

**Kernel không boot (no output):**
- [ ] `cargo bootimage` thành công chưa?
- [ ] QEMU command đúng chưa? (file path, flags)
- [ ] Thêm `-d int,cpu_reset -D qemu.log` và xem log
- [ ] GDT/IDT có được load không? (thiếu có thể gây triple fault ngay)

**Kernel boot nhưng crash sớm:**
- [ ] Đọc panic message — file và dòng nào?
- [ ] Heap đủ lớn chưa? (512KB cho virtio-net)
- [ ] Thứ tự init đúng chưa? (heap trước filesystem)

**Kernel boot nhưng lệnh shell không work:**
- [ ] `serial_println!` ở đầu function để check nó được gọi không
- [ ] Check return value của các fs operations
- [ ] Verify path string (leading slash, case sensitive)

**SMP issues:**
- [ ] Lock có được hold trong interrupt handler không? (deadlock)
- [ ] Shared state có được protect bởi lock không?
- [ ] AP trampoline address đúng chưa? (`0x8000`)

**Network không work:**
- [ ] QEMU command có `-netdev user -device virtio-net-pci` không?
- [ ] Heap >= 512KB không? (RX buffers cần ~100KB)
- [ ] `[virtio-net] Driver ready` có xuất hiện trong log không?
