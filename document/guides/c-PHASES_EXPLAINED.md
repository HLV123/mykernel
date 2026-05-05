# Giải Thích 24 Phases

> Mỗi phase xây dựng trên phase trước. Tài liệu này giải thích mỗi phase làm gì, tại sao cần, và "nếu không có phase này thì sao?"

---

## Giai đoạn A — Xây nền móng (Phases 1–9)

---

### Phase 1–3: Freestanding Binary + VGA + Serial

**Làm gì:**
Tạo binary Rust chạy được mà không cần OS. In "Hello World" ra màn hình VGA và ra serial port.

**Tại sao cần:**
Đây là bước đầu tiên — chứng minh rằng code Rust có thể chạy bare-metal. VGA và serial là 2 cách duy nhất để "nhìn thấy" kernel đang làm gì trong giai đoạn đầu, trước khi có bất kỳ subsystem nào khác.

**Chi tiết kỹ thuật:**
- `#![no_std]` + `#![no_main]` — không có runtime, không có standard library
- VGA text mode: bộ nhớ tại địa chỉ `0xb8000`, mỗi ký tự = 2 bytes (char + color)
- UART serial: port `0x3F8` (COM1), ghi byte ra terminal host
- Custom panic handler: khi crash thì in lỗi và halt CPU

**Nếu không có:**
Không nhìn thấy gì — kernel chạy trong "bóng tối" hoàn toàn, không biết đang làm đúng hay sai.

---

### Phase 4–6: Exceptions + Interrupts + Paging

**Làm gì:**
Bắt CPU exceptions (lỗi) và hardware interrupts. Bật virtual memory (paging).

**Tại sao cần:**

*Exceptions:* Khi CPU gặp lỗi (chia cho 0, truy cập địa chỉ sai), nếu không có handler thì CPU sẽ **triple fault** — máy reset ngay lập tức. Với handler, kernel có thể in lỗi rồi dừng lại thay vì reset im lặng.

*Interrupts:* Keyboard, timer, network card đều dùng interrupt để thông báo. Nếu không enable interrupts, kernel không thể nhận input từ bàn phím hay biết thời gian trôi qua.

*Paging:* Virtual memory cần phải bật từ sớm để mọi địa chỉ pointer trong code đều hoạt động đúng.

**Chi tiết kỹ thuật:**
- IDT: 256 entries, mỗi entry trỏ đến 1 handler function
- Double fault: dùng IST (Interrupt Stack Table) — stack riêng để xử lý lỗi khi stack chính đã overflow
- 4-level paging: PML4 → PDPT → PD → PT → Physical Frame
- Bootloader đã setup paging cơ bản, phase này refinement

**Nếu không có:**
- Không có exception handler → triple fault → máy reset thay vì in lỗi
- Không có interrupts → không nhận được phím bấm, không có timer
- Không có paging → không có memory isolation, không có virtual addresses

---

### Phase 7–9: Heap + Async Executor + Shell

**Làm gì:**
Thêm heap allocator (dùng `Box`, `Vec`, `String`), async executor, và shell tương tác.

**Tại sao cần:**

*Heap:* Kernel cần cấp phát bộ nhớ động — tạo structs mới, build strings, lưu danh sách. Không có heap thì mọi thứ phải có size cố định tại compile time — rất hạn chế.

*Async executor:* Shell cần đọc phím bấm mà không block CPU. Async/await cho phép kernel "ngủ" (HLT) khi không có việc làm, chỉ thức dậy khi có interrupt.

*Shell:* Interface tương tác — không có shell thì kernel chạy xong là dừng, không làm gì được.

**Chi tiết kỹ thuật:**
- Linked-list allocator: quản lý heap bằng danh sách các vùng trống
- Heap ở địa chỉ ảo `0x4444_4440_0000`, mapped vào physical frames
- Async executor: poll futures, sleep bằng HLT khi không có task nào ready
- Keyboard: interrupt handler push scancode vào queue, async task đọc và decode
- Shell: đọc input, parse command, gọi handler

**Nếu không có:**
- Không có heap → không thể dùng Vec, String, Box → code rất cứng nhắc
- Không có async → shell phải spin-wait (lãng phí CPU) hoặc không có shell
- Không có shell → không tương tác được với kernel

---

## Giai đoạn B — User Space + OS Services (Phases 10–18)

---

### Phase 10: Preemptive Scheduler

**Làm gì:**
Kernel có thể chạy nhiều tasks song song, tự động chuyển giữa chúng theo timer.

**Tại sao cần:**
Scheduler là trái tim của multitasking. Không có scheduler, chỉ chạy được 1 task tại 1 thời điểm. Với scheduler, nhiều processes có thể "chạy cùng lúc" — thực ra CPU luân phiên rất nhanh.

**Chi tiết kỹ thuật:**
- Timer interrupt (IRQ0) kích hoạt context switch
- Context switch: lưu tất cả registers của task đang chạy, load registers của task tiếp theo
- `#[unsafe(naked)]`: function không có Rust prologue/epilogue — cần để lưu/restore registers thủ công
- Round-robin: mỗi task được chạy 1 lượt thời gian (time slice) rồi luân phiên

**Nếu không có:**
Một task đang chạy phải tự nhường CPU (cooperative multitasking) — nếu task bị loop vô hạn, toàn bộ hệ thống bị treo.

---

### Phase 11: User Mode (Ring 3)

**Làm gì:**
Kernel có thể chạy code ở Ring 3 (user mode) thay vì Ring 0.

**Tại sao cần:**
Đây là nền tảng của OS isolation. User programs không được có toàn quyền với hardware — nếu có lỗi, chỉ crash program đó, không crash toàn hệ thống.

**Chi tiết kỹ thuật:**
- GDT mở rộng với user code segment (RPL=3) và user data segment
- TSS (Task State Segment): chứa kernel stack pointer — khi syscall xảy ra, CPU switch sang stack này
- SYSCALL/SYSRET: cơ chế fast system call, nhanh hơn INT 0x80
- `IRETQ` với user segment → jump sang Ring 3
- CPU enforce: code Ring 3 không thể dùng privileged instructions (như `HLT`, `IN`, `OUT`)

**Nếu không có:**
Mọi process đều chạy ở Ring 0 — một bug trong user app có thể crash/compromise toàn hệ thống.

---

### Phase 12: Virtual Address Spaces

**Làm gì:**
Mỗi process có page table riêng — không gian địa chỉ ảo độc lập.

**Tại sao cần:**
Hai processes có thể dùng cùng địa chỉ ảo (như `0x400000`) nhưng trỏ vào physical RAM khác nhau. Process A không thể đọc RAM của process B.

**Chi tiết kỹ thuật:**
- Mỗi process có `AddressSpace` với L4 page table riêng
- Kernel entries (higher half) được copy vào page table của mọi process
- Khi context switch: `CR3` register được cập nhật → CPU dùng page table mới
- Physical frames được map vào virtual addresses theo yêu cầu

**Nếu không có:**
Tất cả processes dùng chung 1 page table → process A đọc được RAM của B → security disaster.

---

### Phase 13: ELF Loader

**Làm gì:**
Kernel đọc file ELF binary và load vào memory để chạy.

**Tại sao cần:**
ELF là format của executable files trên Linux (và MyKernel). Không có ELF loader, không thể chạy external programs — chỉ chạy được code nhúng sẵn trong kernel.

**Chi tiết kỹ thuật:**
- ELF64 header: magic bytes, architecture, entry point address
- Program headers (PT_LOAD): mô tả segment nào cần load vào địa chỉ nào
- Permissions: R/W/X flag cho mỗi segment → map vào page table đúng permissions
- Entry point: địa chỉ instruction đầu tiên sau khi load

**Nếu không có:**
Kernel chỉ chạy được code viết thẳng vào kernel binary — không load external programs được.

---

### Phase 14: VFS Layer

**Làm gì:**
Tạo abstraction layer thống nhất cho mọi filesystem. Shell có thể `ls`, `cat`, `write`, `mkdir`.

**Tại sao cần:**
Không có VFS, mỗi filesystem cần interface riêng. Với VFS, application chỉ cần gọi `open()`, `read()`, `write()` — không quan tâm file nằm trong RAM, trên disk, hay là device file.

**Chi tiết kỹ thuật:**
- `FileSystem` trait: interface mà mọi FS phải implement
- `File` trait: interface cho file handles
- Mount table: danh sách `(path, filesystem)` — VFS tìm FS phù hợp khi resolve path
- RamFS: filesystem đơn giản lưu trong RAM — BTreeMap<path, inode>
- DevFS: `/dev/null`, `/dev/zero`, `/dev/serial`
- FdTable: bảng file descriptors per-process

**Nếu không có:**
Shell commands `ls`, `cat`, `write` không tồn tại. Không thể có persistent data trong session.

---

### Phase 15: initramfs (CPIO)

**Làm gì:**
Kernel boot với một filesystem đầy đủ: `/bin`, `/etc`, `/tmp`, các file config.

**Tại sao cần:**
Trước phase này, mỗi lần boot filesystem rỗng hoàn toàn. Initramfs cho phép nhúng sẵn files vào kernel binary — giống cách Linux boot với busybox initrd.

**Chi tiết kỹ thuật:**
- CPIO newc format: header + filename + data cho mỗi entry
- Magic bytes: `070701` ở đầu mỗi header
- `CpioBuilder`: tạo archive từ code Rust
- `load_into_ramfs()`: giải nén CPIO vào VFS

**Nếu không có:**
Mỗi boot phải tự tạo lại files, không có `/etc/hostname`, `/etc/motd`, không có structure mặc định.

---

### Phase 16: Virtio Block Driver

**Làm gì:**
Kernel đọc/ghi được vào virtual disk (disk.img) qua QEMU virtio-blk device.

**Tại sao cần:**
Persistent storage — data không mất khi tắt kernel. Cũng là bước để mount FAT32 filesystem thật.

**Chi tiết kỹ thuật:**
- PCI bus scan: đọc Vendor ID + Device ID qua config space (port `0xCF8`/`0xCFC`)
- Virtio device ID `0x1001` = virtio-blk
- Virtqueue: ring buffer shared giữa kernel và QEMU, dùng DMA
- Request format: `{type, sector, data, status}` — type 0=read, 1=write
- Disk size: đọc từ capability register

**Nếu không có:**
Không có persistent storage — mọi file đều mất khi reboot.

---

### Phase 17: FAT32 Filesystem

**Làm gì:**
Kernel đọc được filesystem FAT32 từ disk image — format phổ biến của USB drives.

**Tại sao cần:**
RamFS lưu trong RAM, mất khi tắt. FAT32 trên disk là persistent. Cũng chứng minh VFS abstraction hoạt động đúng — mount vào `/mnt` và dùng như filesystem bình thường.

**Chi tiết kỹ thuật:**
- BPB (BIOS Parameter Block): metadata ở sector 0 — cluster size, FAT offset, root dir
- FAT table: mảng u32, mỗi entry là cluster tiếp theo (cluster chain)
- 8.3 short name + LFN (Long Filename) entries
- Cluster = đơn vị allocation, thường 4KB-32KB

**Nếu không có:**
Chỉ có RamFS tạm thời. Không đọc được USB drive hay disk image.

---

### Phase 18: Syscalls (Linux ABI)

**Làm gì:**
40 system calls tương thích Linux x86_64: `read`, `write`, `open`, `close`, `mmap`, `getpid`, ...

**Tại sao cần:**
Syscall interface là "ngôn ngữ" mà user programs dùng để xin kernel làm việc. Tương thích Linux ABI nghĩa là (về lý thuyết) có thể chạy binary Linux đơn giản trên MyKernel.

**Chi tiết kỹ thuật:**
- SYSCALL instruction: CPU jump từ Ring 3 → Ring 0 vào handler
- `IA32_LSTAR` MSR: chứa địa chỉ handler
- Convention: `rax` = syscall number, `rdi/rsi/rdx/r10/r8/r9` = arguments
- `arch_prctl(ARCH_SET_FS)`: set FS base register — cần cho musl TLS
- Pointer validation: mọi pointer từ user space phải nằm dưới `0x8000_0000_0000`

**Nếu không có:**
User programs không giao tiếp được với kernel — không mở file, không in ra màn hình được.

---

## Giai đoạn C — Modern OS Features (Phases 19–24)

---

### Phase 19: APIC + SMP

**Làm gì:**
Thay PIC 8259 bằng APIC, parse ACPI để tìm tất cả CPUs, boot các Application Processors.

**Tại sao cần:**
PIC 8259 là chip interrupt controller từ thập niên 80, chỉ support 1 CPU. APIC (Advanced PIC) là chuẩn hiện đại, hỗ trợ multi-core và nhiều tính năng hơn.

**Chi tiết kỹ thuật:**
- Local APIC: mỗi CPU có 1 APIC riêng, memory-mapped tại `0xFEE00000`
- I/O APIC: nhận IRQ từ devices, route đến CPU phù hợp
- ACPI MADT: bảng mô tả toàn bộ processors trong hệ thống
- INIT + SIPI IPI: chuỗi tín hiệu để wake up Application Processors

**Nếu không có:**
Chỉ dùng được 1 CPU lõi, dùng PIC 8259 cũ — không scale lên multi-core.

---

### Phase 20: SMP-safe Locking

**Làm gì:**
Các primitive đồng bộ hóa cho multi-core: SpinLock, RwLock, SeqLock, Once, PerCpu.

**Tại sao cần:**
Khi nhiều CPUs chạy đồng thời và cùng access shared data (như filesystem, network table), cần locking để tránh data corruption.

**Chi tiết kỹ thuật:**
- `SpinLock`: spin-wait (busy loop) + atomic CAS, disable interrupts khi giữ lock
- `RwSpinLock`: nhiều readers hoặc 1 writer
- `SeqLock`: lock-free reads — readers đọc sequence counter trước và sau, retry nếu bị interrupt bởi writer
- `Once`: khởi tạo đúng 1 lần trên SMP — atomic state machine
- `PerCpu`: data riêng cho từng CPU, index bằng APIC ID — không cần lock

**Nếu không có:**
Race conditions trên multi-core → data corruption → crash không đoán trước được.

---

### Phase 21: Virtio Network Driver

**Làm gì:**
Kernel giao tiếp được với virtual ethernet card, gửi/nhận ethernet frames.

**Tại sao cần:**
Đây là layer thấp nhất của networking. Mọi thứ bên trên (ARP, IP, TCP) đều cần driver này để thật sự gửi/nhận dữ liệu.

**Chi tiết kỹ thuật:**
- Device ID `0x1000` = virtio-net
- TX queue: kernel đặt frame vào descriptor ring, notify device
- RX queue: kernel pre-allocate buffers, device điền khi có packet đến
- MAC address: đọc từ device config space tại offset `0x14`
- `internet_checksum()`: RFC 1071 one's complement sum

**Nếu không có:**
Không có networking — toàn bộ phases 22-23 không thể hoạt động.

---

### Phase 22: TCP/IP Stack

**Làm gì:**
Full network stack: ARP, IPv4, ICMP (ping), UDP echo, TCP state machine.

**Tại sao cần:**
Kernel cần hiểu các protocol để communicate với thế giới. Ping, HTTP, SSH đều dựa trên TCP/IP.

**Chi tiết kỹ thuật:**
- ARP: map IPv4 address → MAC address, cache 16 entries
- IPv4: parse header, checksum, route đến đúng protocol handler
- ICMP: tự động reply Echo Request (ping) với Echo Reply
- UDP: stateless, port-based routing, echo server trên port 7
- TCP: state machine CLOSED→LISTEN→SYN_RECEIVED→ESTABLISHED→CLOSE_WAIT→CLOSED

**Nếu không có:**
Không có networking ở level nào — không ping, không HTTP, không socket API.

---

### Phase 23: POSIX Socket API

**Làm gì:**
API socket quen thuộc: `socket()`, `bind()`, `listen()`, `accept()`, `connect()`, `send()`, `recv()`.

**Tại sao cần:**
Đây là interface chuẩn mà mọi network application dùng. Tương thích POSIX nghĩa là code mạng trên Linux có thể porting sang MyKernel với ít thay đổi.

**Chi tiết kỹ thuật:**
- Socket FD bắt đầu từ 100 (tránh conflict với file FDs)
- Socket table: 64 entries
- Port registry: kiểm tra EADDRINUSE khi bind
- `SO_REUSEADDR`: cho phép reuse port sau khi close
- TCP/UDP socket lifecycle khác nhau

**Nếu không có:**
Phải gọi network functions cấp thấp trực tiếp — không có abstraction quen thuộc như `connect()`, `send()`.

---

### Phase 24: Security Hardening

**Làm gì:**
Các tính năng bảo mật: SMEP/SMAP, stack canary, KASLR, pointer validation, capability system, CSPRNG.

**Tại sao cần:**
Kernel phải tự bảo vệ khỏi bugs trong chính nó và khỏi user programs cố tình exploit. Security không phải thứ thêm vào sau — nó phải được design từ đầu.

**Chi tiết kỹ thuật:**
- **SMEP** (CR4 bit 20): CPU fault nếu kernel thực thi code ở user pages — ngăn ret2usr attack
- **SMAP** (CR4 bit 21): CPU fault nếu kernel access user memory không qua STAC/CLAC
- **Stack Canary**: value ngẫu nhiên ở đầu stack frame — nếu bị ghi đè → buffer overflow detected
- **KASLR**: kernel load ở địa chỉ ngẫu nhiên — attacker không biết địa chỉ để exploit
- **Pointer validation**: mọi pointer từ syscall được check nằm trong user space
- **Capabilities**: thay vì root/non-root binary, process có bitmask capabilities cụ thể
- **xoshiro256\*\***: CSPRNG chất lượng cao, seeded từ RDTSC

**Nếu không có:**
Kernel vulnerable với nhiều loại attack: stack overflow, ret2usr, info leak, privilege escalation.

---

## Tóm tắt: Thứ tự phụ thuộc

```
Phase 1-3: VGA + Serial
    │ (cần để debug mọi phase tiếp theo)
    ▼
Phase 4-6: Exceptions + Paging
    │ (cần để mọi thứ không crash)
    ▼
Phase 7-9: Heap + Shell
    │ (cần để có giao diện tương tác)
    ▼
Phase 10-11: Scheduler + Ring 3
    │ (cần để chạy user processes)
    ▼
Phase 12-13: VM + ELF
    │ (cần để load external programs)
    ▼
Phase 14-17: VFS + Filesystem
    │ (cần để có persistent storage)
    ▼
Phase 18: Syscalls
    │ (cần để user programs dùng kernel services)
    ▼
Phase 19-20: APIC + SMP Locking
    │ (cần để scale lên multi-core)
    ▼
Phase 21-23: Network
    │ (cần để communicate với thế giới)
    ▼
Phase 24: Security
    (bảo vệ mọi thứ đã build)
```
