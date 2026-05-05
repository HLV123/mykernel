# MyKernel — Architecture

> Tài liệu mô tả kiến trúc tổng thể của MyKernel — bare-metal x86_64 OS kernel viết bằng Rust.

---

## Tổng quan

MyKernel là một monolithic kernel chạy hoàn toàn ở Ring 0 (kernel mode), không có host OS. Toàn bộ code viết bằng Rust với `#![no_std]` — không dùng standard library, không có runtime. Kernel giao tiếp trực tiếp với hardware qua port I/O, memory-mapped registers, và inline assembly.

```
┌─────────────────────────────────────────────────────────────────┐
│                        User Space (Ring 3)                      │
│                    (ELF processes, syscalls)                    │
├─────────────────────────────────────────────────────────────────┤
│                      SYSCALL Interface                          │
│                  (40 Linux x86_64 ABI calls)                    │
├──────────┬──────────┬──────────┬──────────┬──────────┬──────────┤
│  Shell   │   VFS    │  Network │ Security │   SMP    │ Scheduler│
│          │  Layer   │  Stack   │          │  / APIC  │          │
├──────────┴──────────┴──────────┴──────────┴──────────┴──────────┤
│                       Core Kernel                               │
│          Memory │ Interrupts │ GDT │ Heap │ Async Executor      │
├──────────┬──────────┬──────────┬──────────┬──────────┬──────────┤
│  virtio  │  virtio  │  RamFS   │  DevFS   │  FAT32   │initramfs │
│   -blk   │   -net   │          │          │          │  (CPIO)  │
├──────────┴──────────┴──────────┴──────────┴──────────┴──────────┤
│                        Hardware (x86_64)                        │
│      PCI Bus │ UART │ VGA │ PS/2 │ APIC │ ACPI │ Virtio         │
└─────────────────────────────────────────────────────────────────┘
```

---

## 1. Boot Sequence

```
BIOS/UEFI
    │
    ▼
bootloader v0.9.34
    │  - Loads kernel ELF into memory
    │  - Sets up 64-bit mode
    │  - Builds initial page tables
    │  - Maps physical memory (higher half)
    │  - Calls kernel_main(boot_info)
    ▼
kernel_main()
    │
    ├─► gdt::init()           — Load GDT (segments + TSS)
    ├─► interrupts::init_idt() — Install IDT (exception handlers)
    ├─► PICS.initialize()      — Init 8259 PIC
    ├─► interrupts::enable()   — STI (enable CPU interrupts)
    │
    ├─► memory::init()         — Map physical memory
    ├─► FrameAllocator::init() — Build physical frame allocator
    ├─► allocator::init_heap() — Map heap virtual region
    │
    ├─► fs::init()             — Mount RamFS + DevFS + initramfs
    ├─► process::set_offset()  — Store phys_mem_offset for processes
    ├─► usermode::init_syscalls() — Enable SYSCALL/SYSRET via MSRs
    │
    ├─► drivers::init()        — Probe PCI, init virtio-blk/net
    ├─► net::init()            — Bring up TCP/IP stack
    ├─► security::init()       — Entropy, canary, KASLR, SMEP/SMAP
    │
    ├─► apic::init_lapic()     — Init Local APIC (replace PIC timer)
    ├─► apic::init_ioapic()    — Init I/O APIC
    │
    └─► executor.run()         — Start async executor → shell
```

**Chi tiết từng bước:**

- **GDT** gồm 5 descriptors: null, kernel code (64-bit), kernel data, user code (RPL=3), user data (RPL=3), và 1 TSS descriptor cho kernel stack pointer khi syscall xảy ra.
- **IDT** có 256 entries. Các entries quan trọng: #DE (divide error), #BP (breakpoint), #PF (page fault), #DF (double fault với IST stack riêng), IRQ0 (timer), IRQ1 (keyboard), IRQ4 (serial).
- **Frame allocator** đọc memory map từ bootloader để biết vùng RAM nào available.
- **Heap** được map tại địa chỉ ảo cố định `0x4444_4440_0000`, kích thước configurable trong `allocator.rs`.
- **SYSCALL** được enable bằng cách ghi vào MSRs: `IA32_STAR` (segment selectors), `IA32_LSTAR` (handler address), `IA32_FMASK` (flags mask).

---

## 2. Memory Layout

```
Virtual Address Space (x86_64, 48-bit)
┌─────────────────────────┬──────────────────────────────┐
│ 0x0000_0000_0000_0000   │ User Space                   │
│         ...             │ (processes, stack, heap)     │
│ 0x0000_7FFF_FFFF_FFFF   │                              │
├─────────────────────────┼──────────────────────────────┤
│   (non-canonical hole)  │ 128 TiB gap (CPU enforced)   │
├─────────────────────────┼──────────────────────────────┤
│ 0xFFFF_8000_0000_0000   │ Physical Memory Map          │
│         ...             │ (all RAM mapped here by boot)│
│ 0xFFFF_BFFF_FFFF_FFFF   │                              │
├─────────────────────────┼──────────────────────────────┤
│ 0x4444_4440_0000        │ Kernel Heap                  │
│ (configurable)          │ (512 KiB default)            │
├─────────────────────────┼──────────────────────────────┤
│ 0xFEE0_0000             │ Local APIC (MMIO)            │
│ 0xFEC0_0000             │ I/O APIC (MMIO)              │
└─────────────────────────┴──────────────────────────────┘
```

**Chi tiết:**

- **4-level paging**: PML4 → PDPT → PD → PT → Physical Frame (4KiB pages).
- **Physical memory map**: Bootloader map toàn bộ RAM vào higher half. Kernel dùng offset này để convert physical address → virtual address.
- **Kernel heap**: Linked-list allocator từ crate `linked_list_allocator`. Mỗi allocation tối thiểu 8 bytes, aligned theo type.
- **User address space**: Mỗi process có page table L4 riêng. Kernel entries được copy vào page table của mọi process (higher half kernel). User code chạy ở địa chỉ thấp hơn `0x0000_8000_0000_0000`.
- **Stack canary**: Kernel stack có canary value ở đầu frame — được init từ RDTSC + salt, check tại các điểm quan trọng.

---

## 3. Interrupt & Exception Handling

```
CPU Exception / Hardware IRQ
        │
        ▼
    IDT Entry
        │
        ├── Exception (0-31)
        │       ├── #BP Breakpoint     → print stack frame, continue
        │       ├── #PF Page Fault     → print error, halt
        │       └── #DF Double Fault   → dedicated IST stack, halt
        │
        └── Hardware IRQ (32+)
                ├── IRQ0 Timer         → increment tick counter, send EOI
                ├── IRQ1 Keyboard      → push scancode to queue, wake async task
                └── IRQ4 Serial (COM1) → (available for future use)
```

**Chi tiết:**

- **IST (Interrupt Stack Table)**: Double fault handler dùng IST stack riêng trong TSS để tránh triple fault khi stack overflow xảy ra.
- **PIC 8259**: Master PIC xử lý IRQ 0-7, Slave PIC xử lý IRQ 8-15. Sau khi init APIC, PIC bị mask hết.
- **APIC Timer**: Thay thế PIT sau khi APIC được init. Configured ở ~100 Hz. Mỗi tick tăng global counter `SYSTEM_TICKS`.
- **EOI (End of Interrupt)**: Sau mỗi hardware IRQ handler, phải ghi vào APIC EOI register (0xFEE000B0) để CPU chấp nhận IRQ tiếp theo.
- **Keyboard**: Scancode được push vào `ArrayQueue<u8>` (lock-free, từ crossbeam). Async shell task poll queue này qua `ScancodeStream`.

---

## 4. Filesystem Architecture

```
Application (shell)
        │
        ▼
    fs::read_file() / write_file() / readdir() / mkdir() / remove()
        │
        ▼
┌───────────────────────────────────────────┐
│              VFS Layer                    │
│  MOUNT_TABLE: Vec<MountPoint>             │
│  { path: String, fs: Arc<dyn FileSystem>} │
└───────────┬───────────────────────────────┘
            │ find deepest mount prefix
            ▼
┌───────────┬───────────┬───────────┬───────────┐
│  RamFS    │  DevFS    │  FAT32    │  (future) │
│  at /     │  at /dev  │  at /mnt  │           │
└───────────┴───────────┴───────────┴───────────┘
```

**Chi tiết:**

**VFS (Virtual Filesystem):**
- `FileSystem` trait: `open()`, `create()`, `readdir()`, `mkdir()`, `stat()`, `remove()`.
- `File` trait: `read()`, `write()`, `seek()`, `stat()`.
- Mount table là `Vec<MountPoint>`. Khi resolve path, tìm mount point dài nhất là prefix của path.
- File handle: `Arc<Mutex<dyn File>>` — thread-safe, reference counted.
- File descriptor table: `FdTable` với 256 entries per process.

**RamFS:**
- Storage: `BTreeMap<String, INode>` bảo vệ bởi `SpinLock`.
- INode chứa: `data: Arc<Mutex<Vec<u8>>>` và `file_type: FileType`.
- Hỗ trợ `RegularFile` và `Directory`. Không có hard link hay symlink.

**DevFS:**
- `/dev/null`: đọc trả 0 byte, ghi bỏ qua.
- `/dev/zero`: đọc trả vô hạn zero bytes.
- `/dev/serial`: ghi ra UART COM1.

**initramfs (CPIO):**
- Format: CPIO newc (magic `070701`).
- Parser đọc header → filename → data cho mỗi entry.
- `CpioBuilder` cho phép tạo archive từ code.
- Được load vào RamFS khi boot: tạo directories trước, sau đó tạo files.

**FAT32:**
- Parser đọc BPB (BIOS Parameter Block) để lấy geometry.
- FAT table traversal để follow cluster chains.
- Hỗ trợ 8.3 short names và LFN (Long Filename) entries.
- Mount vào VFS tại `/mnt` khi có virtio-blk device.

---

## 5. Network Stack

```
virtio-net NIC
    │  (raw ethernet frames)
    ▼
┌─────────────────────────────────────────────────┐
│  Ethernet Layer                                 │
│  - Frame parser: dst/src MAC, EtherType         │
│  - Frame builder: build_ethernet_frame()        │
└────────────────────────┬────────────────────────┘
                         │ EtherType dispatch
              ┌──────────┴──────────┐
              ▼                     ▼
         ARP (0x0806)          IPv4 (0x0800)
         - Cache 16 entries    - Header parse/build
         - Reply to requests   - Checksum (RFC 1071)
                                         │ Protocol dispatch
                              ┌──────────┼──────────┐
                              ▼          ▼          ▼
                           ICMP        UDP         TCP
                           - Echo     - Echo      - State machine
                           - Reply    - Port 7    - Listen/Connect
                              │          │          │
                              └──────────┴──────────┘
                                         │
                                         ▼
                              Socket API (POSIX)
                              socket/bind/listen/
                              accept/send/recv/close
```

**Chi tiết:**

**ARP:**
- Cache lưu tối đa 16 entries `(IPv4, MAC)`.
- Tự động reply ARP requests với MAC của kernel.
- `cache_lookup()` trả về MAC nếu có, dùng broadcast nếu chưa có.

**IPv4:**
- Header 20 bytes, TTL=64, ID tăng dần qua `AtomicU16`.
- Checksum tính theo RFC 1071 (one's complement sum).
- Chỉ xử lý unicast packets đến địa chỉ `10.0.2.15`.

**ICMP:**
- Type 8 (Echo Request) → trả Type 0 (Echo Reply).
- Copy Identifier và Sequence Number từ request.
- Kernel tự động reply ping mà không cần shell command.

**UDP:**
- Stateless. Pseudo-header checksum.
- Port 7: echo server — reply lại bất kỳ UDP packet nào.

**TCP:**
- State machine: `CLOSED → LISTEN → SYN_RECEIVED → ESTABLISHED → CLOSE_WAIT → LAST_ACK → CLOSED`.
- Hỗ trợ: SYN, SYN-ACK, ACK, FIN, RST.
- Sequence/Acknowledgement number tracking.
- Chưa có: congestion control, retransmission, sliding window.

**Socket API:**
- FD base = 100 (tránh conflict với file FDs 0-99).
- Socket table: 64 entries.
- `sys_socket()`, `sys_bind()`, `sys_listen()`, `sys_accept()`, `sys_connect()`, `sys_send()`, `sys_recv()`, `sys_sendto()`, `sys_recvfrom()`, `sys_getsockname()`, `sys_getpeername()`, `sys_setsockopt()`, `sys_shutdown()`, `sys_close()`.
- `SO_REUSEADDR` option được hỗ trợ.
- Port registry kiểm tra duplicate binding → trả `EADDRINUSE`.

---

## 6. Syscall Interface

```
User Process (Ring 3)
    │  SYSCALL instruction
    │  rax = syscall number
    │  rdi, rsi, rdx, r10, r8, r9 = arguments
    ▼
syscall_handler() ← MSR IA32_LSTAR
    │
    │  Save registers
    │  Switch to kernel stack (from TSS)
    │
    ▼
syscall_dispatch(rax, rdi, rsi, rdx, r10, r8, r9)
    │
    ├─  0: sys_read(fd, buf, count)
    ├─  1: sys_write(fd, buf, count)
    ├─  2: sys_open(path, flags, mode)
    ├─  3: sys_close(fd)
    ├─  4: sys_stat(path, stat_buf)
    ├─  5: sys_fstat(fd, stat_buf)
    ├─  8: sys_lseek(fd, offset, whence)
    ├─  9: sys_mmap(addr, len, prot, flags, fd, offset)
    ├─ 12: sys_brk(addr)
    ├─ 20: sys_writev(fd, iov, iovcnt)
    ├─ 39: sys_getpid()
    ├─ 60: sys_exit(code)
    ├─ 63: sys_uname(buf)
    ├─ 72: sys_fcntl(fd, cmd, arg)
    ├─ 79: sys_getcwd(buf, size)
    ├─ 96: sys_gettimeofday(tv, tz)
    ├─158: sys_arch_prctl(code, addr)  ← ARCH_SET_FS cho musl TLS
    ├─186: sys_gettid()
    ├─202: sys_futex(...)
    ├─218: sys_set_tid_address(tidptr)
    ├─228: sys_clock_gettime(clockid, tp)
    ├─318: sys_getrandom(buf, buflen, flags)
    └─ ... (40 tổng cộng)
    │
    ▼
    SYSRET → User Process
```

**Chi tiết:**

- **ABI**: Hoàn toàn tương thích Linux x86_64 System V ABI.
- **Pointer validation**: Mọi pointer từ user space được kiểm tra qua `validate_user_ptr()` — phải nằm dưới `0x0000_8000_0000_0000`.
- **FD mapping**: FD 0-2 (stdin/stdout/stderr) map vào DevFS. FD 3+ map vào VFS files. FD 100+ map vào sockets.
- **arch_prctl(ARCH_SET_FS)**: Ghi vào MSR `IA32_FS_BASE` — cần thiết cho musl libc TLS (Thread Local Storage).
- **Global FD table**: 256 entries, shared giữa tất cả processes (simplified — production kernel dùng per-process table).

---

## 7. Security Architecture

```
┌─────────────────────────────────────────────────────────┐
│                  Security Layers                        │
├─────────────────────────────────────────────────────────┤
│  Hardware Level                                         │
│  ├── SMEP (CR4 bit 20): no exec user pages in kernel    │
│  ├── SMAP (CR4 bit 21): no access user mem in kernel    │
│  ├── UMIP (CR4 bit 11): block SGDT/SIDT/SLDT in user    │
│  └── NX/XD (EFER bit 11): data pages non-executable     │
├─────────────────────────────────────────────────────────┤
│  Kernel Level                                           │
│  ├── Stack Canary: RDTSC + 0xDEAD_BEEF_CAFE_BABE        │
│  ├── KASLR: random offset at boot (RDTSC-based)         │
│  └── Hardened Policy: no raw sockets, kptr_restrict     │
├─────────────────────────────────────────────────────────┤
│  Process Level                                          │
│  ├── Pointer Validation: all user ptrs checked          │
│  ├── Capability System: CAP_NET_BIND, CAP_SYS_ADMIN...  │
│  └── copy_from/to_user: safe boundary crossing          │
├─────────────────────────────────────────────────────────┤
│  Cryptography                                           │
│  └── xoshiro256** CSPRNG: seeded from RDTSC at boot     │
└─────────────────────────────────────────────────────────┘
```

**Chi tiết:**

**Stack Canary:**
- Value = `RDTSC XOR 0xDEAD_BEEF_CAFE_BABE`.
- Lưu trong `AtomicU64` global.
- `check_stack_canary(saved)` trả `false` nếu bị ghi đè → panic.

**KASLR:**
- Offset = `(RDTSC & 0x3FF) * 0x200000` — aligned theo 2MB (huge page boundary).
- Mimic concept, không phải full KASLR (bootloader load ở địa chỉ cố định).

**Capability System:**
- Mỗi process có `Capabilities { caps: u64 }` — bitmask.
- Root (uid=0) = `CAP_FULL` (all bits set).
- Unprivileged = `CAP_NONE` (0).
- Kiểm tra: `caps.has(CAP_NET_BIND)` trước khi bind port < 1024.
- Capabilities: `CAP_CHOWN`, `CAP_NET_BIND`, `CAP_NET_RAW`, `CAP_NET_ADMIN`, `CAP_SYS_ADMIN`, `CAP_SYS_REBOOT`, `CAP_DAC_OVERRIDE`, `CAP_SETUID`, `CAP_SETGID`.

**xoshiro256\*\*:**
- 256-bit state, 4 × u64.
- Period: 2^256 - 1.
- Output function: `rotate_left(state[1] * 5, 7) * 9`.
- Dùng cho `getrandom()` syscall và `rand` shell command.

---

## 8. SMP Architecture

```
Boot CPU (BSP)
    │
    ├─► Parse ACPI RSDP → XSDT → MADT
    │       └── Collect Local APIC entries (CPU list)
    │
    ├─► Init Local APIC (BSP)
    │       ├── Disable 8259 PIC (mask all)
    │       ├── Set Spurious Vector = 0xFF
    │       └── Configure APIC Timer @ ~100 Hz
    │
    ├─► Init I/O APIC
    │       └── Route IRQ1 (keyboard) → Vector 33
    │
    └─► For each AP (Application Processor):
            ├── Write trampoline code to 0x8000 (below 1MB)
            ├── Send INIT IPI → wait 10ms
            ├── Send SIPI IPI (start at 0x08) → wait 200µs
            ├── Send SIPI IPI again (safety) → wait 200µs
            └── AP boots: setup GDT/IDT → signal BSP → idle

Application Processor (AP)
    │
    ├─► Read entry point from AP_DATA.entry (AtomicU64)
    ├─► Setup local GDT and IDT
    ├─► Enable Local APIC
    ├─► Set ONLINE_COUNT++
    └─► Call ap_main() → idle loop (HLT)
```

**Chi tiết:**

- **ACPI MADT**: Parser tìm RSDP tại địa chỉ `0xE0000-0xFFFFF` hoặc từ XSDT pointer. Đọc MADT để lấy danh sách LAPIC entries.
- **Trampoline**: Code 16-bit real mode tại địa chỉ thấp, bootstrap AP vào 64-bit protected mode.
- **AP_DATA**: Shared struct `{ entry: AtomicU64, ready: AtomicBool }` — BSP ghi entry point, AP đọc và jump.
- **CPU_COUNT**: Tổng số CPU detect từ MADT.
- **ONLINE_COUNT**: Số CPU đã boot thành công, tăng dần khi mỗi AP khởi động xong.
- **PerCpu**: Mỗi CPU có data riêng không cần lock, index bằng APIC ID.

---

## 9. Async Executor

```
executor.run()
    │
    ├─► Poll all ready tasks
    │       │
    │       └─► Task::poll()
    │               │
    │               ├── Ready(output) → remove task
    │               └── Pending → store waker, suspend
    │
    ├─► If no ready tasks:
    │       └─► HLT (sleep until next interrupt)
    │
    └─► Interrupt arrives → wake relevant task → loop
```

**Chi tiết:**

- **Task**: Wrapper quanh `Pin<Box<dyn Future>>`.
- **TaskId**: `AtomicU64` counter, unique per task.
- **Waker**: Custom `TaskWaker` implement `ArcWake` — push TaskId vào `task_queue` khi wake.
- **task_queue**: `Arc<ArrayQueue<TaskId>>` — lock-free MPSC queue.
- **Executor loop**: Pop TaskId từ queue → lookup task → poll. Nếu queue rỗng → `HLT`.
- **Shell task**: Duy nhất 1 task trong executor. Poll UART COM1 trong spin loop (không suspend) để đọc input.
- **Keyboard stream**: `ScancodeStream` implement `futures::Stream`. `AtomicWaker` được register, wake khi interrupt handler push scancode.

---

## 10. Driver Architecture

```
PCI Bus Scan
    │  (port I/O: 0xCF8 config address, 0xCFC config data)
    │
    ├─► Scan bus 0, device 0-31, function 0-7
    │       Read Vendor ID + Device ID
    │
    ├─► Device 0x1001 (virtio-blk) found?
    │       └─► virtio_blk::init()
    │               ├── Read BAR0 → I/O base address
    │               ├── Negotiate features
    │               ├── Setup virtqueue (descriptor ring)
    │               └─► Expose: read_sector(), write_sector(), num_sectors()
    │
    └─► Device 0x1000 (virtio-net) found?
            └─► virtio_net::init()
                    ├── Read BAR0 → I/O base address
                    ├── Read MAC from config space
                    ├── Setup TX virtqueue + RX virtqueue
                    ├── Allocate RX buffers (→ requires heap space)
                    └─► Expose: send_packet(), recv_packet(), get_mac()
```

**Chi tiết:**

**PCI:**
- Config space access qua legacy port I/O (Type 1).
- Address register: `bus << 16 | device << 11 | function << 8 | offset | 0x80000000`.
- Đọc BARs để lấy I/O base address của device.

**Virtio (legacy, spec 0.9):**
- Virtqueue: descriptor ring + available ring + used ring.
- Descriptor: `{ addr, len, flags, next }`.
- TX: kernel ghi descriptor → update available ring → notify device (write 0 to Queue Notify).
- RX: kernel pre-fill descriptors → device ghi vào khi có packet → kernel poll used ring.

**virtio-blk:**
- Sector size: 512 bytes.
- Request format: `{ type, sector, data, status }`.
- Type 0 = read, type 1 = write.
- Detect disk size từ capacity register (8 bytes tại offset 0x14).

**virtio-net:**
- Header: `VirtioNetHdr { flags, gso_type, hdr_len, gso_size, csum_start, csum_offset }`.
- RX buffer: 1526 bytes (1500 MTU + 14 Ethernet header + 12 virtio header).
- MAC đọc từ device config space tại offset 0x14.
- Features negotiated: VIRTIO_NET_F_MAC (bit 5).

---

## 11. Module Dependency Map

```
main.rs
    │
    ├── lib.rs (core init)
    │    ├── gdt.rs
    │    ├── interrupts.rs ──────────────────► task/keyboard.rs
    │    ├── memory.rs
    │    └── allocator.rs
    │
    ├── fs/
    │    ├── mod.rs ──── vfs.rs ◄──── ramfs.rs
    │    │                       ◄──── devfs.rs
    │    │                       ◄──── fat32.rs
    │    └── initramfs.rs ──────► ramfs.rs
    │
    ├── drivers/
    │    ├── pci.rs
    │    ├── virtio.rs ◄── virtio_blk.rs
    │    │             ◄── virtio_net.rs ──► net/
    │    └── mod.rs
    │
    ├── net/
    │    ├── mod.rs ──── arp.rs
    │    │          ──── ip.rs
    │    │          ──── icmp.rs
    │    │          ──── udp.rs
    │    │          ──── tcp.rs
    │    └── socket.rs
    │
    ├── security.rs
    ├── smp.rs ──────────────────────────────► apic.rs
    ├── sync.rs (SpinLock, RwLock, SeqLock)
    ├── syscall.rs ──────────────────────────► fs/, net/socket.rs
    ├── usermode.rs
    ├── scheduler.rs
    ├── process.rs
    ├── elf_loader.rs
    ├── shell.rs ────────────────────────────► fs/, net/, security.rs
    └── task/
         ├── executor.rs
         ├── keyboard.rs
         └── simple_executor.rs
```

---

## 12. Source Files Summary

| File | Dòng (approx) | Mô tả |
|------|--------------|-------|
| `src/main.rs` | ~120 | Entry point, boot sequence |
| `src/lib.rs` | ~100 | Library crate, test infrastructure |
| `src/gdt.rs` | ~120 | GDT, TSS setup |
| `src/interrupts.rs` | ~200 | IDT, exception/IRQ handlers |
| `src/memory.rs` | ~150 | Frame allocator, page table mapper |
| `src/allocator.rs` | ~60 | Heap allocator wrapper |
| `src/scheduler.rs` | ~150 | Round-robin preemptive scheduler |
| `src/process.rs` | ~120 | Address space, process struct |
| `src/elf_loader.rs` | ~180 | ELF64 parser and loader |
| `src/syscall.rs` | ~600 | 40 Linux-compatible syscalls |
| `src/usermode.rs` | ~100 | SYSCALL/SYSRET handler |
| `src/shell.rs` | ~450 | Interactive shell, 18 commands |
| `src/vga_buffer.rs` | ~150 | VGA text mode + serial mirror |
| `src/serial.rs` | ~60 | UART COM1 driver |
| `src/apic.rs` | ~280 | Local APIC + I/O APIC |
| `src/smp.rs` | ~470 | ACPI parser, AP boot, CPU table |
| `src/sync.rs` | ~350 | SpinLock, RwLock, SeqLock, Once, PerCpu |
| `src/security.rs` | ~430 | CSPRNG, canary, KASLR, capabilities |
| `src/fs/vfs.rs` | ~290 | VFS trait, mount table |
| `src/fs/ramfs.rs` | ~230 | In-memory filesystem |
| `src/fs/devfs.rs` | ~120 | /dev pseudo-devices |
| `src/fs/initramfs.rs` | ~280 | CPIO parser + builder |
| `src/fs/fat32.rs` | ~380 | FAT32 filesystem |
| `src/drivers/pci.rs` | ~100 | PCI bus scanner |
| `src/drivers/virtio.rs` | ~80 | Virtio queue primitives |
| `src/drivers/virtio_blk.rs` | ~230 | Block storage driver |
| `src/drivers/virtio_net.rs` | ~280 | Ethernet NIC driver |
| `src/net/mod.rs` | ~120 | Network stack coordinator |
| `src/net/arp.rs` | ~100 | ARP cache + reply |
| `src/net/ip.rs` | ~120 | IPv4 layer |
| `src/net/icmp.rs` | ~80 | ICMP echo |
| `src/net/tcp.rs` | ~250 | TCP state machine |
| `src/net/udp.rs` | ~90 | UDP layer |
| `src/net/socket.rs` | ~400 | POSIX Socket API |
| `src/task/executor.rs` | ~120 | Async task executor |
| `src/task/keyboard.rs` | ~80 | Async keyboard stream |
| **Total** | **~7,500** | |

---

## 13. Key Design Decisions

**Tại sao Rust?**
Memory safety tại compile time — không có buffer overflows, use-after-free, hay data races theo compiler đảm bảo. Đặc biệt quan trọng cho kernel code nơi một lỗi nhỏ có thể crash toàn bộ hệ thống.

**Tại sao monolithic?**
Đơn giản hơn microkernel cho mục đích học tập. Mọi subsystem giao tiếp trực tiếp qua function calls thay vì IPC.

**Tại sao virtio?**
Virtio là interface chuẩn cho QEMU virtual devices — đơn giản hơn emulate real hardware (e1000, AHCI) nhưng vẫn dạy đủ concepts về DMA, virtqueue, device negotiation.

**Tại sao async/await?**
Shell cần đọc keyboard events mà không blocking CPU. Async executor cho phép `HLT` khi không có việc làm — tiết kiệm CPU hơn spin loop. Cũng là cơ hội demonstrate Rust async trong no_std context.

**Tại sao Linux ABI cho syscalls?**
Tương thích với musl libc và các binary Linux đơn giản. Developer quen thuộc với interface này. Dễ test với existing tools.
