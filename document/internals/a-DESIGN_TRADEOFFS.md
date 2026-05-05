# Design Tradeoffs

> Mỗi quyết định thiết kế trong MyKernel đều có lý do và trade-off. Tài liệu này giải thích tại sao chọn A thay vì B.

---

## Monolithic vs Microkernel

### Chọn: Monolithic

**Monolithic kernel:** Tất cả subsystems (filesystem, network, drivers) chạy trong 1 address space, Ring 0. Components gọi nhau qua function calls trực tiếp.

**Microkernel:** Kernel chỉ có IPC, scheduling, memory management. Filesystem, network, drivers là user-space servers giao tiếp qua IPC.

### Tại sao Monolithic cho MyKernel?

**Đơn giản hơn:** Không cần IPC layer, không cần capability-based access control giữa servers, không cần shared memory protocols. Code trực tiếp, dễ hiểu.

**Dễ debug hơn:** Mọi subsystem trong cùng address space → stack trace đầy đủ, không mất context qua IPC boundary.

**Tốc độ:** Không có IPC overhead. Filesystem call → function call trực tiếp, không phải context switch sang server khác.

**Trade-off:**
- Một driver lỗi có thể crash toàn kernel (không isolation)
- Kernel address space phức tạp hơn
- Khó live-update driver khi đang chạy

**So sánh:** Linux, Windows NT, macOS XNU đều monolithic hoặc hybrid. Serenity OS là monolithic Rust kernel. Redox OS là microkernel Rust kernel — kiến trúc rất khác, phức tạp hơn nhiều.

---

## Virtio Legacy (0.9) vs Virtio Modern (1.x)

### Chọn: Virtio Legacy (0.9)

**Legacy spec:**
- Device register truy cập qua PCI I/O BAR (port I/O)
- Feature negotiation đơn giản hơn
- QEMU support từ phiên bản cũ

**Modern spec (1.x):**
- Truy cập qua MMIO hoặc PCI MMIO BAR
- Feature bits mở rộng (64-bit)
- Packed virtqueue (hiệu quả hơn)
- Notification mechanisms khác

### Tại sao Legacy?

**Đơn giản hơn:** Port I/O dễ implement hơn MMIO BAR. Không cần parse PCI capabilities structure.

**QEMU compat:** QEMU vẫn support legacy cho `-device virtio-blk-pci` và `-device virtio-net-pci`.

**Học tập:** Concept giống nhau giữa legacy và modern, chỉ khác transport. Hiểu legacy → hiểu modern dễ hơn.

**Trade-off:**
- QEMU có thể deprecate legacy trong tương lai
- Hiệu năng thấp hơn (packed virtqueue không available)
- Không support trên non-x86 (port I/O là x86-specific)

**Để upgrade lên Modern:** Cần parse PCI capabilities, tìm Common Config BAR, implement MMIO access. Virtqueue logic (descriptor ring, available/used) không thay đổi.

---

## Linked-list Allocator vs Slab vs Buddy

### Chọn: Linked-list Allocator

**Linked-list:** Danh sách holes, first-fit allocation, coalesce khi free.

**Slab allocator:** Pre-allocated pools cho size cố định (32B, 64B, 128B, ...). Fast cho frequent small allocations.

**Buddy allocator:** Binary tree của 2^N size blocks. O(log N) allocation và free, natural fragmentation prevention.

### Tại sao Linked-list?

**Minimal implementation:** Dùng crate `linked_list_allocator` — 100 dòng code, zero dependencies, proven correct.

**Đủ cho learning:** Kernel cần allocator để bootstrap các subsystems khác. Allocator phức tạp hơn không phải mục tiêu chính của project.

**Trade-off:**
- O(N) allocation (scan toàn bộ free list)
- Fragmentation có thể xảy ra sau nhiều alloc/free
- Không efficient cho nhiều small allocations

**Production thay thế:** Linux dùng slab allocator (kmalloc) + buddy allocator cho page frames. Rust embedded projects hay dùng `talc` hay `good-memory-allocator`.

---

## Global FD Table vs Per-process FD Table

### Chọn: Global FD Table

```rust
// MyKernel: 1 global table, 256 entries
static GLOBAL_FD_TABLE: Mutex<FdTable> = Mutex::new(FdTable::new());

// Linux: per-process table, referenced counted
struct Task { files: Arc<FilesStruct> }
struct FilesStruct { fd_array: Vec<Option<Arc<File>>> }
```

### Tại sao Global?

**Đơn giản:** Không cần per-process state cho FDs khi chưa có real process isolation.

**Syscall implementation:** `sys_read(fd, ...)` chỉ cần lookup global table, không cần biết current process.

**Trade-off:**
- Processes có thể thấy FDs của nhau (không secure)
- FD 3 trong process A và process B là cùng 1 file
- `fork()` semantics không thể implement đúng (child cần copy parent's FD table)

**Khi nào cần per-process:**
- Khi implement real process isolation
- Khi `fork()`/`exec()` cần hoạt động đúng
- Khi `close-on-exec` (FD_CLOEXEC) cần support

---

## Cooperative Async vs Preemptive Threading cho Shell

### Chọn: Cooperative Async Executor

**Async executor:** Shell là 1 future. Khi không có input, future return `Poll::Pending`, executor chạy `HLT`. Interrupt arrives → executor polls lại.

**Preemptive thread:** Shell là 1 kernel thread. Scheduler preempt bằng timer interrupt. Shell block trong `read()` syscall.

### Tại sao Async?

**CPU efficiency:** `HLT` = CPU ngủ cho đến interrupt. Không waste cycles spin-waiting.

**No stack per task:** Async task không cần dedicated stack. Future state machine được compile vào struct. Nhưng MyKernel hiện tại dùng serial polling (spin loop) — mất đi advantage này.

**Rust support:** `async/await` native trong Rust, `futures::Stream` cho keyboard events.

**Trade-off:**
- Async code khó debug hơn (stack trace không intuitive)
- Serial polling loop hiện tại không fully async (spin cho đến khi có input)
- Không thể "block" tự nhiên — phải restructure code thành state machines

**Note về MyKernel shell cụ thể:**

Shell hiện tại dùng serial polling — KHÔNG fully async:
```rust
// Shell read_line: spin loop polling UART
let key = loop {
    if let Some(c) = try_read_serial() { break c; }
    for _ in 0..10000 { core::hint::spin_loop(); }
};
```

Đây là trade-off giữa đơn giản và correctness — fully async với serial input cần UART interrupt handler và queue tương tự keyboard. Chọn polling vì đơn giản hơn.

---

## Linux ABI vs Custom ABI cho Syscalls

### Chọn: Linux x86_64 ABI

**Linux ABI:** Syscall numbers, argument conventions, error codes, struct layouts theo Linux specification.

**Custom ABI:** Tự define syscall numbers, có thể optimize cho kernel cụ thể.

### Tại sao Linux ABI?

**Developer familiarity:** Mọi developer biết Linux syscalls. Không cần học thêm interface mới.

**Toolchain compat:** musl libc, strace, ltrace, GDB đều biết Linux ABI.

**Testing:** Có thể so sánh behavior với Linux để verify implementation.

**Trade-off:**
- Bị ràng buộc bởi Linux interface dù không implement đầy đủ
- Một số syscalls phức tạp không cần thiết cho MyKernel
- Stub syscalls return wrong values có thể confuse musl

**Những chỗ không tương thích:**
- `fork()`: stub, không thực sự fork
- `mmap()`: chỉ anonymous mmap, không file-backed
- `futex()`: stub, không có futex wait queue
- Signal handling: không implement

---

## APIC Timer vs PIT Timer

### Chọn: APIC Timer (sau khi PIC init)

**PIT (Programmable Interval Timer):** Legacy 8253/8254 chip, frequency 1.193182 MHz, output vào IRQ0.

**APIC Timer:** Per-CPU timer trong Local APIC, không có fixed frequency (bus-dependent), output vào vector trong IDT.

### Tại sao APIC Timer?

**Per-CPU:** Mỗi CPU có timer riêng → mỗi CPU có thể schedule independently (cần cho SMP scheduler).

**Higher resolution:** APIC timer frequency = CPU bus frequency / divisor, thường cao hơn PIT.

**No PIC dependency:** Sau khi APIC init và PIC masked, PIT interrupt không đến được. APIC timer đảm bảo timer luôn fire.

**Trade-off:**
- Phức tạp hơn (cần calibrate APIC timer frequency dùng PIT)
- Không có fixed frequency — phải measure
- Cần ACPI/CPUID để detect

**Calibration trong MyKernel (simplified):**
```rust
// Start APIC timer với large initial count
lapic.write(LAPIC_TIMER_ICR, 0xFFFFFFFF);
// Đo số APIC ticks trong 1 PIT tick (~10ms)
// Tính initial count cho ~100Hz
```

---

## VFS Trait Objects vs Enum Dispatch

### Chọn: Trait Objects (`dyn FileSystem`)

**Trait objects:**
```rust
pub trait FileSystem: Send + Sync { ... }
// Stored as Arc<dyn FileSystem> — vtable dispatch
```

**Enum dispatch:**
```rust
enum AnyFs {
    Ram(RamFs),
    Dev(DevFs),
    Fat32(Fat32Fs),
}
impl AnyFs {
    fn open(&self, ...) { match self { ... } }
}
```

### Tại sao Trait Objects?

**Extensibility:** Thêm filesystem mới không cần sửa enum. Chỉ cần implement trait.

**Plugin-like:** Mount table chứa `Vec<MountPoint>` với `Arc<dyn FileSystem>` — heterogeneous.

**Trade-off enum:**
- `match` nhanh hơn vtable call
- Compiler có thể inline match arms
- Nhưng: thêm filesystem mới → sửa enum và mọi match → breaking change

**Trade-off trait object:**
- Virtual dispatch overhead (~1 cache miss)
- Compiler không inline qua trait boundary (thường)
- Dynamic dispatch không predictable cho CPU branch predictor

Trong kernel context với I/O bound operations, virtual dispatch overhead là negligible.

---

## Stack Canary: RDTSC vs RDRAND

### Chọn: RDTSC

```rust
let tsc: u64 = unsafe {
    let val;
    core::arch::asm!("rdtsc", out("rax") val, out("rdx") _);
    val
};
let canary = tsc ^ 0xDEAD_BEEF_CAFE_BABE;
```

**RDTSC:** Timestamp counter, tăng dần mỗi CPU cycle. Không predictable từ bên ngoài.

**RDRAND:** Hardware RNG (Intel), cryptographically secure.

### Tại sao RDTSC?

**Universal support:** RDRAND chỉ có trên CPU Intel Ivy Bridge+ và AMD Ryzen+. QEMU TCG không emulate RDRAND. RDTSC universal hơn.

**Đủ cho stack canary:** Canary không cần cryptographically secure — chỉ cần unpredictable đủ để ngăn attacker guess. RDTSC + XOR với magic constant đủ.

**Trade-off:**
- RDTSC có thể bị read bởi user space (`RDTSC` instruction available ở Ring 3)
- Attacker biết approximate boot time → có thể narrow down canary value
- Production kernel nên dùng RDRAND khi available, fallback RDTSC

**Improvement:**
```rust
pub fn get_entropy() -> u64 {
    // Try RDRAND first
    let mut rand = 0u64;
    let success: u8;
    unsafe {
        core::arch::asm!(
            "rdrand {0}",
            "setc {1}",
            out(reg) rand,
            out(reg_byte) success,
        );
    }
    if success != 0 { return rand; }
    
    // Fallback to RDTSC
    let tsc: u64;
    unsafe { core::arch::asm!("rdtsc", out("rax") tsc, out("rdx") _); }
    tsc ^ 0xDEAD_BEEF_CAFE_BABE
}
```

---

## Tóm tắt Tradeoff Matrix

| Quyết định | Chọn | Đơn giản | Hiệu năng | Extensibility | Production-ready |
|-----------|------|----------|-----------|---------------|-----------------|
| Kernel architecture | Monolithic | ✅ | ✅ | ⚠️ | ✅ (Linux) |
| Virtio spec | Legacy 0.9 | ✅ | ⚠️ | ⚠️ | ⚠️ |
| Heap allocator | Linked-list | ✅ | ❌ | ✅ | ⚠️ |
| FD table | Global | ✅ | ✅ | ❌ | ❌ |
| Shell I/O | Async+polling | ⚠️ | ⚠️ | ✅ | ⚠️ |
| Syscall ABI | Linux compat | ✅ | ✅ | ✅ | ✅ |
| Timer | APIC | ⚠️ | ✅ | ✅ | ✅ |
| FS dispatch | Trait objects | ✅ | ⚠️ | ✅ | ✅ |
| Canary entropy | RDTSC | ✅ | ✅ | - | ⚠️ |
