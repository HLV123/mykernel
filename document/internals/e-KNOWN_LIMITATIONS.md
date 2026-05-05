# Known Limitations

> Những gì MyKernel chưa implement, tại sao, và hệ quả khi cố dùng.

---

## Memory

### Không có page reclaim

**Vấn đề:** Heap chỉ có thể grow, không shrink. Một khi memory được allocate, kernel giữ mãi ngay cả khi không cần nữa. Frame allocator không có `free_frame()`.

**Hệ quả:** Long-running kernel sessions có thể OOM dần dần. Không có swap để recover.

**Workaround:** Restart kernel khi OOM. Tăng `HEAP_SIZE` trước khi OOM xảy ra.

**Implement nếu cần:**
```rust
// Cần thêm vào BootInfoFrameAllocator:
pub fn free_frame(&mut self, frame: PhysFrame) {
    self.free_list.push(frame);
}

// Và track free list:
struct BootInfoFrameAllocator {
    memory_map: &'static MemoryMap,
    next: usize,
    free_list: Vec<PhysFrame>,  // returned frames
}
```

### Không có demand paging

**Vấn đề:** Tất cả pages phải được map trước khi access. Không có lazy allocation — không có "allocate on first touch".

**Hệ quả:**
- ELF loader phải map tất cả PT_LOAD segments upfront
- BSS không thể được zero-initialized lazily
- Stack không thể grow on demand

**Implement nếu cần:** Page fault handler phải biết VMA (Virtual Memory Area) context — cấu trúc mô tả các vùng hợp lệ trong address space. Khi page fault xảy ra, check VMA, allocate frame, map, continue.

### Không có Copy-on-Write

**Vấn đề:** `fork()` phải copy toàn bộ address space thay vì share pages với COW.

**Hệ quả:** `fork()` bị stub — không actually fork. `exec()` cũng bị stub.

**Implement nếu cần:** Set pages read-only khi fork. Page fault handler detect COW page, copy frame, set writable.

---

## Process Management

### Không có real fork/exec

**Hiện tại:**
```rust
fn sys_fork() -> i64 { 0 }      // always return "child" PID 0
fn sys_execve(...) -> i64 { -1 } // ENOSYS
```

**Hệ quả:** Không thể spawn child processes. Shell scripts không chạy được. Dynamic linker không load được.

**Để implement fork:**
1. Clone current process struct
2. Copy page table (COW optional)
3. Copy FD table
4. Duplicate stack và return address
5. Return 0 in child, child PID in parent

### Không có signal handling

**Hiện tại:** `sys_rt_sigaction`, `sys_sigprocmask` đều là stubs trả `0`.

**Hệ quả:**
- `Ctrl+C` trong user process không kill process
- No SIGSEGV → process không được notified khi access violation
- No SIGPIPE → write to closed pipe không fail gracefully

**Implement nếu cần:** Signal frame trên user stack, `sigreturn` syscall, per-process signal handlers và masks.

### Không có process groups, sessions, terminal control

**Hệ quả:** `tcgetpgrp`, `tcsetpgrp` fail. Shell job control không work. Daemon double-fork không work.

### Global FD table (không per-process)

**Hệ quả:**
- FD 3 trong "process A" và "process B" là cùng 1 file
- `close(3)` trong một process đóng file cho tất cả
- `fork()` semantics sai (child nên inherit copy của FD table)

---

## Filesystem

### RamFS không persistent

**Vấn đề:** Mọi thứ trong `/tmp`, files tự tạo, đều mất khi kernel tắt.

**Workaround:** Dùng FAT32 trên virtio-blk để lưu persistent data.

### Không có permissions

**Vấn đề:** Tất cả files readable và writable bởi tất cả. Không có `chmod`, `chown`. `stat()` trả mode `0755` hardcoded.

**Hệ quả:** musl `access()` check fail nếu expect specific permissions.

### Không có hard links hay symlinks

**Hệ quả:**
- `ln` command không thể implement
- `/usr/bin/python3 → python3.11` symlink không work
- `POSIX` compliant programs có thể fail nếu expect symlinks

### Không có directory timestamps

`stat()` trả mtime = 0 cho tất cả files.

### FAT32 read-only

**Hiện tại:** FAT32 driver chỉ đọc, không ghi. `create()` và `write()` trong Fat32Fs trả `PermissionDenied`.

**Implement write support:** Cần track dirty clusters, update FAT chain khi file grow, implement directory entry creation.

---

## Networking

### TCP không có retransmission

**Vấn đề:** Nếu packet bị lost, connection sẽ hang mãi mãi. Không có timeout, không có RTO (Retransmission Timeout).

**Hệ quả:** Chỉ hoạt động tốt trong loopback hoặc lossless QEMU network. Real-world network = unreliable.

**Implement nếu cần:**
```rust
struct TcpConnection {
    // ...
    unacknowledged: VecDeque<(u32, Vec<u8>, Instant)>,  // seq, data, send_time
    rto: Duration,  // retransmission timeout
}

fn check_retransmit(&mut self) {
    let now = get_time();
    for (seq, data, sent_at) in &self.unacknowledged {
        if now - sent_at > self.rto {
            self.retransmit(seq, data);
        }
    }
}
```

### Không có TCP flow control

**Vấn đề:** Receiver buffer size không được advertised. Sender gửi không giới hạn.

**Hệ quả:** Có thể overwhelm slow receiver.

### Không có congestion control

**Vấn đề:** Không có slow start, congestion avoidance, fast retransmit.

**Hệ quả:** Sẽ congest network nếu có nhiều connections.

### Không có IPv6

Chỉ IPv4. EtherType `0x86DD` bị ignore.

### Ping không nhận reply trên Windows QEMU

**Vấn đề:** QEMU user-mode (SLiRP) trên Windows không forward ICMP packets từ host vào guest.

**Workaround:**
- Dùng Linux host với TAP interface
- Hoặc chấp nhận giới hạn này — kernel TCP/IP stack vẫn đúng, chỉ là transport layer (QEMU networking) bị giới hạn

### UDP echo chỉ trên port 7

**Vấn đề:** UDP echo server hardcoded trên port 7. Không có general UDP socket server.

**Hiện tại:** Chỉ có built-in echo. Không thể implement UDP server trong shell.

---

## Security

### KASLR không phải real KASLR

**Vấn đề:** `KASLR_OFFSET` là giá trị tính từ RDTSC, nhưng kernel thực sự được load ở địa chỉ cố định bởi bootloader. Offset này không ảnh hưởng gì đến layout thật.

**Hệ quả:** Security score tính +15 điểm cho KASLR nhưng protection thật không có.

**Real KASLR:** Cần bootloader load kernel ở địa chỉ ngẫu nhiên, hoặc kernel relocate bản thân sau boot.

### Stack canary không compiler-generated

**Vấn đề:** Canary được check manually, không phải compiler inject vào mọi stack frame.

**Hệ quả:** Chỉ detect overflow nếu code explicitly gọi `check_stack_canary()`.

**Real stack protector:** Cần `-Z stack-protector=all` compiler flag (unstable Rust), và linker symbols:
```c
extern uint64_t __stack_chk_guard;
void __stack_chk_fail(void) { panic!(); }
```

### SMEP/SMAP = off trên QEMU TCG

**Vấn đề:** QEMU TCG (software emulation) không support SMEP/SMAP in CR4.

**Workaround:** Dùng KVM (`-enable-kvm` trên Linux host) để enable hardware virtualization. Khi đó SMEP/SMAP được enforce thật sự.

### Không có Spectre/Meltdown mitigations

**Vấn đề:** Không có KPTI (Kernel Page Table Isolation), không có retpoline, không có IBRS.

**Hệ quả:** Vulnerable trên real hardware. Không quan trọng trong QEMU (không có speculative execution attacks).

---

## Shell

### Serial input polling (không fully async)

**Vấn đề:** Shell dùng spin loop để poll UART:
```rust
let key = loop {
    if let Some(c) = try_read_serial() { break c; }
    for _ in 0..10000 { core::hint::spin_loop(); }
};
```

**Hệ quả:** CPU không idle hoàn toàn khi chờ input. Tiêu thụ ~100% CPU.

**Fix:** Implement UART interrupt handler, push bytes vào queue, async read từ queue (giống keyboard).

### Shell không có job control

Không có `Ctrl+C` kill, không có `Ctrl+Z` suspend, không có background jobs (`cmd &`).

### Command history không có

Mỗi lần boot, không có history. Không có arrow key navigation.

### Tab completion không có

Không có autocomplete cho paths hay commands.

---

## SMP

### AP chỉ idle sau boot

**Hiện tại:** Application Processors boot thành công nhưng chỉ chạy `HLT` loop. Không có work stealing, không có task migration.

**Để tận dụng multi-core:** Cần per-CPU run queues, load balancer, và task migration mechanism.

### Không có NUMA awareness

Tất cả memory allocation không biết NUMA topology. Có thể gây performance degradation trên NUMA systems.

---

## Compatibility

### Không chạy được real Linux binaries

Dù syscall interface tương thích về số và convention, hầu hết real binaries fail vì:
- Thiếu dynamic linker (`ld.so`)
- Thiếu vDSO
- Thiếu `/proc`, `/sys` filesystems
- `clone()` không work (threads)
- Signal handling không đủ

**Exception:** Statically linked binaries với minimal syscalls có thể work.

### musl libc partially works

musl initialize thành công nhưng nhiều operations fail:
- `pthread_create` fail (no clone)
- File I/O works với files trong initramfs
- `printf` works (via `write` syscall)
- `malloc` works (via `brk`/`mmap`)
