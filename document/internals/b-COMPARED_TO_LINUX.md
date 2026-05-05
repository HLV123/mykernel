# So Sánh với Linux Kernel

> Đặt MyKernel vào context của Linux — cùng concept, implementation khác nhau. Giúp developer quen Linux hiểu MyKernel nhanh hơn, và hiểu đâu là simplification.

---

## Boot Sequence

### Linux

```
BIOS/UEFI
    │
    ▼
Bootloader (GRUB/systemd-boot)
    │  - Loads bzImage (compressed kernel)
    │  - Decompresses kernel in-place
    │  - Sets up boot parameters (struct boot_params)
    ▼
startup_64 (arch/x86/kernel/head_64.S)
    │  - Assembly entry, setup page tables
    │  - Enable long mode nếu 32-bit entry
    ▼
x86_64_start_kernel()
    │  - early_idt_handler_array
    │  - setup ACPI, NUMA, SMP
    │  - call start_kernel()
    ▼
start_kernel()
    │  - trap_init(), mm_init(), sched_init()
    │  - rest_init() → kernel_init thread
    ▼
kernel_init()
    │  - Mount initramfs, run /init
    └  → userspace PID 1 (systemd/init)
```

### MyKernel

```
BIOS
    │
    ▼
bootloader crate (0.9.34)
    │  - Minimal x86_64 bootloader
    │  - Setup long mode, page tables
    │  - Map physical memory
    │  - Call kernel_main(boot_info)
    ▼
kernel_main()
    │  - Linear init, no threads
    └  → executor.run() → shell
```

**Khác biệt chính:**
- Linux có multi-stage boot (BIOS → GRUB → kernel init)
- Linux boot vào PID 1 → systemd. MyKernel boot vào async shell
- Linux dùng compressed kernel image. MyKernel dùng raw ELF qua bootimage

---

## Memory Management

### Linux: Multi-layer System

```
Physical: Buddy Allocator (alloc_pages, __get_free_pages)
    │  2^N page blocks, per-NUMA-node, per-zone
    ▼
Slab: SLAB/SLUB/SLOB (kmalloc, kmem_cache_alloc)
    │  Fixed-size caches, per-CPU caches
    ▼
Virtual: vmalloc (non-contiguous physical, contiguous virtual)
    │  Dùng cho module loading, large allocations
    ▼
User: mmap, brk (via VMA — Virtual Memory Area)
    │  Demand paging, COW, swap
```

### MyKernel: Single Layer

```
Physical: BootInfoFrameAllocator (alloc only, no free)
    │  Simple iteration over memory map
    ▼
Heap: Linked-list allocator (Box/Vec/Arc)
    │  First-fit, coalesce on free
    ▼
No vmalloc, no swap, no COW
```

**Linux features MyKernel không có:**
- Demand paging (pages not loaded until accessed)
- Copy-on-write fork
- Swap/paging to disk
- NUMA-aware allocation
- Per-CPU slab caches
- Transparent huge pages (THP)
- Memory compaction
- OOM killer

---

## Process Model

### Linux

```
task_struct: ~1KB struct mô tả 1 thread
  - mm_struct: virtual address space
  - files_struct: FD table (per-process, ref-counted)
  - signal_struct: signal handlers
  - cred: credentials (uid, gid, capabilities)
  - sched_entity: scheduler info
  - ...

fork() = copy task_struct + mm (COW) + files (if not CLONE_FILES)
exec() = replace mm, load new ELF, reset signals
clone() = configurable sharing (threads vs processes)
```

### MyKernel

```
Process: minimal struct
  - AddressSpace: page table
  - scheduler state

Không có:
  - signal handling
  - real fork() (stub)
  - process groups, sessions
  - /proc/<pid> entries
  - capabilities per-process (global policy)
```

---

## Filesystem

### Linux VFS

```
struct super_block  — filesystem instance
struct inode        — file metadata + ops
struct dentry       — directory cache entry (name → inode)
struct file         — open file handle (per FD)

struct inode_operations  { create, lookup, link, unlink, mkdir, ... }
struct file_operations   { read, write, llseek, ioctl, mmap, ... }
struct super_operations  { alloc_inode, destroy_inode, sync_fs, ... }

Dentry cache: LRU cache của path → inode lookups
Inode cache: LRU cache của inodes
Page cache: file contents cached in memory pages
```

### MyKernel VFS

```
trait FileSystem  { open, create, readdir, mkdir, stat, remove }
trait File        { read, write, seek, stat }

Không có:
  - Dentry cache
  - Inode cache  
  - Page cache (reads go directly to storage)
  - Hard links
  - Symlinks
  - File permissions / access control
  - Extended attributes (xattr)
```

**Linux `vfs_read` vs MyKernel:**

Linux:
```c
ssize_t vfs_read(struct file *file, char __user *buf, size_t count, loff_t *pos) {
    // Check permissions
    // Check file is readable
    // Call file->f_op->read() hoặc new_sync_read()
    // Update atime
    // ...
}
```

MyKernel:
```rust
fn sys_read(fd: u64, buf_ptr: u64, count: u64) -> i64 {
    validate_user_ptr(buf_ptr, count as usize);
    let file = get_file(fd)?;
    let x = match file.lock().read(buf) { ... };
    x
}
```

---

## Networking

### Linux Network Stack

```
Socket layer (AF_INET, SOCK_STREAM)
    │  sys_socket, sys_bind, sys_connect, ...
    ▼
Protocol layer (struct proto)
    │  tcp_sendmsg, tcp_recvmsg, ...
    ▼
IP layer (ip_output, ip_input)
    │  Routing, fragmentation, NAT
    ▼
Neighbour subsystem (ARP)
    │  arp_resolve, neigh_output
    ▼
Device layer (struct net_device)
    │  ndo_start_xmit, ...
    ▼
Driver (e1000, virtio_net, ...)
```

**Linux extras:**
- Netfilter/iptables (firewall hooks)
- Traffic control (tc, qdisc)
- Bridge, VLAN, tunnel
- IPv6
- Routing tables (FIB)
- Network namespaces

### MyKernel Network Stack

```
Socket API (socket/bind/listen/accept/send/recv)
    │  Single file: src/net/socket.rs
    ▼
Protocol handlers (src/net/tcp.rs, udp.rs, icmp.rs)
    │  State machines, no retransmit
    ▼
IP layer (src/net/ip.rs)
    │  No routing (single NIC, single IP)
    ▼
ARP (src/net/arp.rs)
    │  Simple cache, 16 entries
    ▼
virtio-net driver
```

**Linux TCP vs MyKernel TCP:**

| Feature | Linux | MyKernel |
|---------|-------|---------|
| State machine | Full (11 states) | Simplified (7 states) |
| Retransmission | Yes (RTO, RACK) | No |
| Congestion control | CUBIC, BBR, ... | No |
| Window scaling | Yes | No |
| Nagle algorithm | Yes | No |
| TIME_WAIT | Yes | No |
| TCP options | Full | SYN only |
| SACK | Yes | No |

---

## Scheduler

### Linux CFS (Completely Fair Scheduler)

```
Red-black tree, sorted by vruntime (virtual runtime)
  - Smallest vruntime = next to run
  - vruntime += actual_time × (1024 / weight)
  - Priority → weight → slower vruntime increase = more CPU time

Groups:
  - cgroup-based resource control
  - Bandwidth throttling

Real-time:
  - SCHED_FIFO, SCHED_RR
  - Hard deadlines

Multicore:
  - Per-CPU run queues
  - Load balancing between CPUs
  - NUMA-aware
```

### MyKernel Scheduler

```
Round-robin:
  - Vec<Task> (no priority)
  - Timer interrupt → next task
  - Context switch: save/restore 6 registers

Không có:
  - Priority
  - Preemption trong kernel (không preemptible)
  - Load balancing
  - NUMA awareness
  - Real-time scheduling
```

---

## Device Drivers

### Linux Driver Model

```
struct platform_device, struct pci_device → probe()
  ↓
struct device_driver { probe, remove, suspend, resume }
  ↓
sysfs entries (/sys/bus/pci/devices/...)
  ↓
udev events → user-space device management
  ↓
/dev entries (major:minor number)
```

**Linux virtio-net driver** (drivers/net/virtio_net.c):
- ~3000 dòng code
- NAPI (New API) polling để giảm interrupt overhead
- Multiple TX/RX queues (multi-queue)
- XDP (eXpress Data Path) support
- GSO/TSO offload

**MyKernel virtio-net:**
- ~280 dòng code
- Single TX/RX queue
- Polling based
- No offload

---

## Syscall Implementation

### Linux sys_read

```c
SYSCALL_DEFINE3(read, unsigned int, fd, char __user *, buf, size_t, count)
{
    return ksys_read(fd, buf, count);
}

ssize_t ksys_read(unsigned int fd, char __user *buf, size_t count)
{
    struct fd f = fdget_pos(fd);
    ssize_t ret = -EBADF;
    
    if (f.file) {
        loff_t pos, *ppos = file_ppos(f.file);
        if (ppos) {
            pos = *ppos;
            ppos = &pos;
        }
        ret = vfs_read(f.file, buf, count, ppos);
        if (ret >= 0 && ppos)
            f.file->f_pos = pos;
        fdput_pos(f);
    }
    return ret;
}
```

Lưu ý:
- `fdget_pos` lấy file + lock position (concurrent reads)
- Cập nhật `f_pos` sau khi đọc
- `fdput_pos` release lock

### MyKernel sys_read

```rust
fn sys_read(fd: u64, buf_ptr: u64, count: u64) -> i64 {
    if !validate_user_ptr(buf_ptr, count as usize) { return -EFAULT; }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, count as usize) };
    let file = match get_file(fd) { Some(f) => f, None => return -EBADF };
    let x = match file.lock().read(buf) {
        Ok(n) => n as i64,
        Err(FsError::EndOfFile) => 0,
        Err(_) => EBADF,
    }; x
}
```

Đơn giản hơn nhiều — không có concurrent position tracking, không fdput_pos.

---

## Security

### Linux Security Features

- **LSM (Linux Security Modules)**: AppArmor, SELinux, Seccomp
- **Capabilities**: 41 capabilities, per-thread effective/permitted/inheritable sets
- **Namespaces**: pid, net, mnt, user, uts, ipc — isolation
- **cgroups**: resource limits per group
- **Landlock**: filesystem sandboxing
- **KASLR**: kernel + module + stack ASLR
- **SMEP/SMAP**: hardware enforced
- **Spectre/Meltdown mitigations**: KPTI, retpoline, ...
- **Stack protector**: compiler-generated canaries
- **FORTIFY_SOURCE**: safer string functions

### MyKernel Security

- Stack canary (manual, not compiler-generated)
- KASLR offset (simplified, not real load address randomization)
- Pointer validation (all syscall pointers checked)
- Capability system (simplified, not per-process sets)
- SMEP/SMAP (detect + enable if CPU supports)

**Chênh lệch lớn nhất:** No namespace isolation, no LSM, no Spectre mitigations, no KPTI.

---

## Những Gì MyKernel Thiếu (Có Chủ Ý)

### Không thể implement trong scope project:

| Feature | Tại sao không có |
|---------|-----------------|
| Page cache | Cần phức tạp invalidation, writeback, VFS integration |
| Demand paging | Cần page fault handler biết VMA context |
| COW fork | Cần per-process page table + COW bits |
| Signal handling | Cần sigframe, restorer, async-signal-safe |
| Network routing | Cần routing table, multiple interfaces |
| TCP retransmit | Cần timer wheel, RTT estimation |
| Swap | Cần page reclaim, LRU, swapfile management |
| Module loading | Cần ELF relocations, export symbols, vermagic |
| Power management | Cần ACPI S3, CPPC, frequency scaling |

### Có thể implement nhưng là scope mở rộng:

| Feature | Complexity |
|---------|-----------|
| Per-process FD table | Medium |
| Real KASLR | Medium (cần bootloader cooperation) |
| Compiler stack protector | Medium (cần linker symbols) |
| TCP retransmit | High |
| IPv6 | High |
| NVMe driver | High |
| USB HID keyboard | High |
