# MyKernel Internals

> Chi tiết implementation nội bộ — data structures, memory layout, invariants, và những chỗ cần chú ý khi sửa code.

---

## GDT Layout

```
Index  Descriptor        DPL  Type
─────────────────────────────────────────────
0      Null              -    (required by CPU)
1      Kernel Code       0    64-bit, Execute/Read
2      Kernel Data       0    64-bit, Read/Write
3      User Code         3    64-bit, Execute/Read, Conforming
4      User Data         3    64-bit, Read/Write
5      TSS (low 64)      0    64-bit TSS descriptor (16 bytes total)
6      TSS (high 64)     0    (continuation of TSS descriptor)
```

**Segment Selectors:**
- Kernel CS = `0x08` (index 1, RPL=0)
- Kernel DS = `0x10` (index 2, RPL=0)
- User CS   = `0x2B` (index 5, RPL=3, TI=0) — `(5 << 3) | 3`
- User DS   = `0x23` (index 4, RPL=3)
- TSS       = `0x28` (index 5 × 8)

**TSS Structure (chọn lọc):**
```rust
#[repr(C, packed)]
struct Tss {
    _reserved_1: u32,
    rsp: [u64; 3],      // RSP0, RSP1, RSP2 — kernel stack khi Ring change
    _reserved_2: u64,
    ist: [u64; 7],      // IST1-IST7 — dedicated stacks cho exceptions
    _reserved_3: u64,
    _reserved_4: u16,
    iomap_base: u16,
}
```

`RSP0` = kernel stack top, được load khi SYSCALL xảy ra (CPU switch từ Ring 3 → Ring 0).

`IST1` = dedicated stack cho double fault handler — phải valid ngay cả khi kernel stack bị overflow.

---

## IDT Entry Format

```
Bits 127:96  : Reserved
Bits  95:80  : Offset[63:32]
Bits  79:64  : Offset[31:16]
Bits  63:48  : (zero)
Bit      47  : Present
Bits  46:45  : DPL (Descriptor Privilege Level)
Bit      44  : (zero)
Bits  43:40  : Gate Type (0xE = interrupt gate, 0xF = trap gate)
Bits  39:35  : IST (Interrupt Stack Table index, 0 = don't switch)
Bits  34:32  : (zero)
Bits  31:16  : Segment Selector
Bits   15:0  : Offset[15:0]
```

**Interrupt gate vs Trap gate:**
- Interrupt gate: CPU clear IF flag (disable interrupts) khi vào handler
- Trap gate: CPU giữ IF flag nguyên

MyKernel dùng interrupt gate cho hầu hết handlers (tránh nested interrupts), trap gate cho breakpoint (cho phép interrupt trong debugger).

---

## Syscall Fast Path — MSR Configuration

```
IA32_STAR (0xC0000081):
  Bits 63:48 = SS selector for SYSRET to 64-bit (user SS = STAR[63:48] + 8)
  Bits 47:32 = CS selector for SYSRET to 64-bit (user CS = STAR[47:32] + 16)
  Bits 31:0  = (kernel CS << 16) | 0 — CS for SYSCALL

IA32_LSTAR (0xC0000082):
  = address of syscall handler

IA32_FMASK (0xC0000084):
  = RFLAGS bits to clear on SYSCALL
  MyKernel: 0x200 (IF flag — disable interrupts in syscall handler)
```

**SYSCALL instruction behavior:**
1. Save RIP → RCX
2. Save RFLAGS → R11, clear bits in FMASK
3. Load CS from STAR[47:32], SS from STAR[47:32]+8
4. Jump to LSTAR

**SYSRET instruction behavior:**
1. Load RIP from RCX
2. Load RFLAGS from R11
3. Load CS from STAR[63:48]+16, SS from STAR[63:48]+8
4. Switch to Ring 3

**Registers NOT preserved across SYSCALL:**
- RCX (saved RIP), R11 (saved RFLAGS)
- Convention: RAX = return value

---

## Heap Virtual Address Layout

```
HEAP_START = 0x_4444_4440_0000
HEAP_SIZE  = configurable (default: 512KB)

Physical frames:
  BootInfoFrameAllocator cấp frames từ memory map
  1 frame (4KB) per page → 512KB = 128 frames

Page Table Mapping (trong init_heap):
  for each page in [HEAP_START, HEAP_START + HEAP_SIZE):
    map_page(virt_page, frame, PRESENT | WRITABLE)
```

**Linked-list allocator internals:**

```
Heap memory (sau khi một số allocations):

[Header|----------|Header|free......|Header|---|Header|free...........]
       ^used 128B         ^free 4096B       ^used 64B  ^free rest

Header = HoleInfo { size: usize, next: Option<NonNull<HoleInfo>> }
```

Allocation: tìm first-fit hole đủ lớn, split nếu cần.
Deallocation: coalesce với adjacent holes.

**Alignment:** mọi allocation align theo `core::mem::align_of::<usize>()` = 8 bytes.

---

## Virtqueue Implementation Detail

```rust
// Descriptor Ring (256 entries)
#[repr(C)]
struct VirtqDesc {
    addr:  u64,    // physical address của buffer
    len:   u32,    // length
    flags: u16,    // VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE
    next:  u16,    // next descriptor index (nếu có NEXT flag)
}

// Available Ring (driver → device)
#[repr(C)]
struct VirtqAvail {
    flags: u16,
    idx:   u16,          // số lần driver đã add entries
    ring:  [u16; 256],   // descriptor indices
}

// Used Ring (device → driver)
#[repr(C)]
struct VirtqUsed {
    flags: u16,
    idx:   u16,           // số lần device đã process entries
    ring:  [VirtqUsedElem; 256],
}

#[repr(C)]
struct VirtqUsedElem {
    id:  u32,  // descriptor index
    len: u32,  // bytes written (cho RX)
}
```

**Notify Device:** ghi queue index vào Queue Notify port:
```rust
unsafe {
    let mut port = Port::<u16>::new(io_base + VIRTIO_PCI_QUEUE_NOTIFY);
    port.write(queue_idx as u16);
}
```

**Poll Used Ring:**
```rust
fn poll(&mut self) -> Option<(u16, u32)> {
    if self.last_used_idx == self.used_ring.idx {
        return None;  // không có gì mới
    }
    let elem = &self.used_ring.ring[self.last_used_idx as usize % 256];
    self.last_used_idx = self.last_used_idx.wrapping_add(1);
    Some((elem.id as u16, elem.len))
}
```

---

## Physical Memory Map Constants

```rust
// BIOS/legacy reserved — không cấp phát
const BIOS_RESERVED_RANGES: &[(u64, u64)] = &[
    (0x000E0000u64, 0x000FFFFFu64),  // BIOS ROM
    (0x00080000u64, 0x0009FFFFu64),  // Partially reserved
];

// VGA framebuffer
const VGA_BUFFER: u64 = 0xB8000;

// APIC MMIO (không cấp phát physical frames vùng này)
const LAPIC_BASE:  u64 = 0xFEE00000;
const IOAPIC_BASE: u64 = 0xFEC00000;
```

---

## Stack Canary — Implementation Details

```rust
static STACK_CANARY: AtomicU64 = AtomicU64::new(0);

pub fn init_stack_canary() {
    let tsc: u64 = unsafe {
        let val;
        core::arch::asm!("rdtsc", out("rax") val, out("rdx") _);
        val
    };
    let canary = tsc ^ 0xDEAD_BEEF_CAFE_BABE;
    STACK_CANARY.store(canary, Ordering::Relaxed);
}
```

**Giá trị canary mỗi boot khác nhau** vì RDTSC phụ thuộc vào thời điểm boot.

**Hạn chế của implementation hiện tại:**
- Canary chỉ check explicitly, không auto-inject vào stack frames như GCC `-fstack-protector`
- Để có stack protector thật, cần linker symbols `__stack_chk_guard` và `__stack_chk_fail`
- Rust có hỗ trợ qua `-Z stack-protector=all` (unstable)

---

## SMP AP Boot Trampoline

```
Physical address 0x8000 (trampoline):
┌─────────────────────────────────────────┐
│ 16-bit real mode code:                  │
│   cli                                   │
│   lgdt [gdt_ptr]    ; load temp GDT     │
│   mov eax, cr0                          │
│   or  eax, 1        ; set PE bit        │
│   mov cr0, eax      ; enter protected   │
│   jmp far 0x10:pm32 ; flush CS          │
│                                         │
│ 32-bit protected mode:                  │
│   mov ax, 0x18      ; data segment      │
│   mov ds/es/ss, ax                      │
│   ; enable PAE, load PML4               │
│   mov eax, cr4                          │
│   or  eax, (1<<5)   ; PAE               │
│   mov cr4, eax                          │
│   ; set EFER.LME                        │
│   ; load CR3 (BSP's page table)         │
│   mov eax, cr0                          │
│   or  eax, (1<<31)  ; PG                │
│   mov cr0, eax      ; enter long mode   │
│   jmp far 0x08:long_mode                │
│                                         │
│ 64-bit long mode:                       │
│   ; setup stack (unique per AP)         │
│   ; call ap_main()                      │
└─────────────────────────────────────────┘
```

**AP_DATA struct** shared giữa BSP và APs:
```rust
struct ApData {
    entry:     AtomicU64,    // address của ap_main()
    ready:     AtomicBool,   // AP set = 1 khi online
    stack_top: AtomicU64,    // stack pointer cho AP
}
```

**INIT + SIPI sequence:**
```
BSP gửi INIT IPI:
  ICR (Interrupt Command Register):
    Delivery Mode = 5 (INIT)
    Level = 1 (Assert)
    Destination = target APIC ID

BSP wait 10ms

BSP gửi SIPI IPI:
  ICR:
    Delivery Mode = 6 (SIPI)
    Vector = 0x08 (trampoline at 0x8000 = 0x08 << 12)
    Destination = target APIC ID

BSP wait 200µs, gửi lại SIPI lần 2 (safety)

AP boot, gọi ap_main():
  - Setup local GDT/IDT
  - Enable Local APIC
  - ONLINE_COUNT.fetch_add(1)
  - Idle loop (HLT)
```

---

## Per-CPU Data

```rust
pub struct PerCpu<T: Copy> {
    values: [T; MAX_CPUS],  // MAX_CPUS = 8
}

impl<T: Copy> PerCpu<T> {
    pub fn get(&self) -> T {
        let id = crate::apic::lapic_id() as usize;
        self.values[id]
    }
    pub fn set(&mut self, val: T) {
        let id = crate::apic::lapic_id() as usize;
        self.values[id] = val;
    }
}
```

APIC ID được dùng làm index — không cần lock vì mỗi CPU chỉ access index của mình. Đây là pattern quan trọng cho scalable kernel data.

---

## SeqLock — Lock-free Read Path

```rust
pub struct SeqLock<T: Copy> {
    sequence: AtomicU32,
    data: UnsafeCell<T>,
}

impl<T: Copy> SeqLock<T> {
    pub fn read(&self) -> T {
        loop {
            let seq1 = self.sequence.load(Ordering::Acquire);
            if seq1 & 1 != 0 {
                // Odd = write in progress, retry
                core::hint::spin_loop();
                continue;
            }
            let val = unsafe { *self.data.get() };
            let seq2 = self.sequence.load(Ordering::Acquire);
            if seq1 == seq2 {
                return val;  // consistent read
            }
            // seq changed = write happened during read, retry
        }
    }

    pub fn write(&self, val: T) {
        // Increment to odd (signals write in progress)
        self.sequence.fetch_add(1, Ordering::Release);
        unsafe { *self.data.get() = val; }
        // Increment to even (write done)
        self.sequence.fetch_add(1, Ordering::Release);
    }
}
```

Dùng cho system timer (`get_ticks()`) — reads không cần lock, chỉ writes mới exclusive.

---

## CPIO newc Format — Byte Layout

```
Field        Offset  Size  Description
───────────────────────────────────────────────────────
magic        0       6     "070701"
ino          6       8     inode number (hex ASCII)
mode         14      8     file mode (octal perms + type)
uid          22      8     user ID
gid          30      8     group ID
nlink        38      8     number of hard links
mtime        46      8     modification time
filesize     54      8     file data size in bytes
devmajor     62      8     device major
devminor     70      8     device minor
rdevmajor    78      8     (for special files)
rdevminor    86      8     (for special files)
namesize     94      8     filename length (including null)
check        102     8     (always 0 for newc)
             110     -     filename (null-terminated)
             +pad          padding to 4-byte boundary
             -             file data
             +pad          padding to 4-byte boundary
```

**End-of-archive entry:** filename = `"TRAILER!!!"`, filesize = 0.

**File type từ mode bits:**
```
0o040000 = directory
0o100000 = regular file
0o120000 = symlink
0o060000 = block device
0o020000 = character device
```

---

## FAT32 — BPB Offsets

```
Offset  Size  Field
──────────────────────────────────────
0       3     Jump boot code + NOP
3       8     OEM name
11      2     Bytes per sector
13      1     Sectors per cluster
14      2     Reserved sector count
16      1     Number of FATs
17      2     Root entry count (0 for FAT32)
19      2     Total sectors 16 (0 for FAT32)
21      1     Media type
22      2     FAT size 16 (0 for FAT32)
24      2     Sectors per track
26      2     Number of heads
28      4     Hidden sectors
32      4     Total sectors 32
──── FAT32 specific ────────────────
36      4     FAT size 32 (sectors per FAT)
40      2     Ext flags
42      2     FS version
44      4     Root cluster (usually 2)
48      2     FS info sector
50      2     Backup boot sector
```

**Cluster number to LBA:**
```
data_start = reserved_sectors + (num_fats × fat_size)
lba = data_start + (cluster - 2) × sectors_per_cluster
```

---

## ELF64 — Header và Program Header

```
ELF64 Header (64 bytes):
Offset  Size  Field
0       4     Magic: 0x7F 'E' 'L' 'F'
4       1     EI_CLASS: 2 = 64-bit
5       1     EI_DATA: 1 = little-endian
6       1     EI_VERSION: 1
7       1     EI_OSABI: 0 = System V
16      2     e_type: 2 = ET_EXEC
18      2     e_machine: 0x3E = x86-64
24      8     e_entry: virtual address of entry point
32      8     e_phoff: offset of program headers
40      8     e_shoff: offset of section headers
48      4     e_flags
52      2     e_ehsize: 64
54      2     e_phentsize: 56
56      2     e_phnum: number of program headers
...

ELF64 Program Header (56 bytes):
0       4     p_type: 1=PT_LOAD, 3=PT_INTERP, 6=PT_PHDR
4       4     p_flags: PF_X=1, PF_W=2, PF_R=4
8       8     p_offset: offset in file
16      8     p_vaddr: virtual address
24      8     p_paddr: physical address (usually = vaddr)
32      8     p_filesz: size in file
40      8     p_memsz: size in memory (>= filesz, extra = BSS)
48      8     p_align: alignment (must be power of 2)
```

**Loading PT_LOAD segments:**
1. Map virtual pages [p_vaddr, p_vaddr + p_memsz)
2. Copy p_filesz bytes từ file[p_offset]
3. Zero fill [p_vaddr + p_filesz, p_vaddr + p_memsz) — đây là BSS

---

## Syscall Table — Số và Handlers

```
Number  Name              Handler
──────────────────────────────────────────────
0       read              sys_read(fd, buf, count)
1       write             sys_write(fd, buf, count)
2       open              sys_open(path, flags, mode)
3       close             sys_close(fd)
4       stat              sys_stat(path, stat_buf)
5       fstat             sys_fstat(fd, stat_buf)
8       lseek             sys_lseek(fd, offset, whence)
9       mmap              sys_mmap(addr, len, prot, flags, fd, off)
11      munmap            sys_munmap(addr, len) — stub, returns 0
12      brk               sys_brk(addr)
16      ioctl             sys_ioctl(fd, request, arg)
17      pread64           sys_pread64(fd, buf, count, offset)
20      writev            sys_writev(fd, iov, iovcnt)
21      access            sys_access(path, mode) — stub, returns 0
33      dup2              sys_dup2(oldfd, newfd)
39      getpid            sys_getpid()
56      clone             sys_clone(...) — stub
57      fork              sys_fork() — stub
59      execve            sys_execve(path, argv, envp) — stub
60      exit              sys_exit(code)
61      wait4             sys_wait4(...) — stub
63      uname             sys_uname(buf)
72      fcntl             sys_fcntl(fd, cmd, arg)
78      getdents          sys_getdents(fd, buf, count) — stub
79      getcwd            sys_getcwd(buf, size)
80      chdir             sys_chdir(path)
83      mkdir             sys_mkdir(path, mode)
87      unlink            sys_unlink(path)
96      gettimeofday      sys_gettimeofday(tv, tz)
102     getuid            sys_getuid() — returns 0 (root)
104     getgid            sys_getgid() — returns 0
107     geteuid           sys_geteuid() — returns 0
108     getegid           sys_getegid() — returns 0
110     getppid           sys_getppid() — returns 0
111     getpgrp           sys_getpgrp() — returns 0
158     arch_prctl        sys_arch_prctl(code, addr)
186     gettid            sys_gettid()
202     futex             sys_futex(...) — stub
218     set_tid_address   sys_set_tid_address(tidptr)
228     clock_gettime     sys_clock_gettime(clockid, tp)
231     exit_group        sys_exit_group(code)
257     openat            sys_openat(dirfd, path, flags, mode)
262     newfstatat        sys_newfstatat(dirfd, path, buf, flags)
290     accept4           sys_accept4(fd, addr, addrlen, flags)
302     prlimit64         sys_prlimit64(pid, resource, new, old)
318     getrandom         sys_getrandom(buf, len, flags)
```

**Pointer validation — tất cả user pointers phải pass:**
```rust
pub fn validate_user_ptr(ptr: u64, len: usize) -> bool {
    ptr != 0
    && ptr < USER_SPACE_TOP  // 0x0000_8000_0000_0000
    && ptr.checked_add(len as u64).map_or(false, |end| end <= USER_SPACE_TOP)
}
```

---

## Invariants Quan Trọng

**Heap phải init trước filesystem:**
RamFS dùng `BTreeMap<String, INode>` — cần heap. Nếu init FS trước heap → panic.

**GDT phải load trước IDT:**
IDT entries dùng kernel CS selector — phải valid trong GDT trước khi IDT được load.

**APIC phải init sau PIC:**
PIC phải được mask trước khi APIC timer được enable — nếu không, cả 2 cùng fire timer interrupt.

**Interrupts phải disable khi giữ SpinLock:**
```rust
pub fn lock(&self) -> SpinLockGuard<T> {
    x86_64::instructions::interrupts::disable();  // PHẢI disable trước
    while self.locked.compare_exchange(...).is_err() { spin_loop(); }
    SpinLockGuard { lock: self }
}
```
Nếu không disable: interrupt handler cố lấy lock đang held → deadlock.

**Virtio RX buffers phải pre-allocate:**
QEMU điền packet vào buffer được kernel cung cấp sẵn. Nếu không có buffer → QEMU drop packet.

**Page table phải flush sau khi thay đổi:**
`invlpg` instruction hoặc reload CR3 để flush TLB sau khi map/unmap pages.
