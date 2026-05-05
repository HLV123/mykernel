# Deep Dive: Filesystem

> Giải thích cách MyKernel tổ chức và quản lý files — từ VFS abstraction layer đến RamFS, initramfs, và FAT32.

---

## Tại Sao Cần Filesystem?

Không có filesystem, data chỉ tồn tại ở dạng raw bytes ở địa chỉ nhất định. Filesystem cung cấp:

- **Tên**: thay vì "địa chỉ 0x12345", bạn truy cập "/etc/hostname"
- **Cấu trúc**: directories chứa files và directories khác
- **Metadata**: kích thước, loại, permissions
- **Abstraction**: đọc file từ RAM, disk, hay device đều dùng cùng API

---

## VFS — Virtual Filesystem Layer

VFS là "người phiên dịch" giữa application và filesystem thật. Application chỉ gọi `open()`, `read()`, `write()` — không cần biết data nằm trong RAM hay trên disk.

### FileSystem Trait

```rust
pub trait FileSystem: Send + Sync {
    fn open(&self, path: &str, flags: OpenFlags) 
        -> FsResult<Arc<Mutex<dyn File>>>;
    fn create(&self, path: &str) 
        -> FsResult<Arc<Mutex<dyn File>>>;
    fn readdir(&self, path: &str) 
        -> FsResult<Vec<DirEntry>>;
    fn mkdir(&self, path: &str) -> FsResult<()>;
    fn stat(&self, path: &str) -> FsResult<Stat>;
    fn remove(&self, path: &str) -> FsResult<()>;
}
```

Bất kỳ filesystem nào implement trait này đều hoạt động với VFS.

### File Trait

```rust
pub trait File: Send + Sync {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize>;
    fn write(&mut self, buf: &[u8]) -> FsResult<usize>;
    fn seek(&mut self, offset: i64, whence: SeekWhence) -> FsResult<u64>;
    fn stat(&self) -> FsResult<Stat>;
}
```

### Mount Table

```rust
struct MountPoint {
    path: String,
    fs: Arc<dyn FileSystem>,
}

static MOUNT_TABLE: Mutex<Vec<MountPoint>> = ...;
```

Khi resolve path, VFS tìm mount point dài nhất là prefix:

```
Path: "/etc/hostname"

Mount table:
  "/" → RamFS
  "/dev" → DevFS
  "/mnt" → Fat32Fs

Longest prefix match: "/" → dùng RamFS
Relative path: "etc/hostname"
```

---

## RamFS — Filesystem trong RAM

RamFS là filesystem đơn giản nhất — lưu tất cả trong RAM.

### Cấu trúc Data

```rust
struct INode {
    data: Arc<Mutex<Vec<u8>>>,  // nội dung file
    file_type: FileType,         // RegularFile hoặc Directory
}

struct RamFs {
    inodes: SpinLock<BTreeMap<String, INode>>,
}
```

Toàn bộ filesystem là 1 `BTreeMap<String, INode>` — key là path chuẩn hóa.

### Normalize Path

```
"/tmp/hello.txt"     → "tmp/hello.txt"
"/tmp/"              → "tmp"
"/etc//hostname"     → "etc/hostname"
```

Không có thư mục thật — "directory" chỉ là INode với `file_type = Directory`. `readdir` filter các entries có cùng prefix.

### Ví dụ: Tạo và đọc file

```
write("/tmp/hello.txt", "Hello World"):
  1. create("/tmp/hello.txt")
  2. inodes.insert("tmp/hello.txt", INode { data: b"Hello World", type: Regular })

read("/tmp/hello.txt"):
  1. open("/tmp/hello.txt")
  2. inodes.get("tmp/hello.txt") → INode
  3. return INode.data
```

### Giới hạn

- Không persistent: mất khi tắt kernel
- Không có hard links, symlinks
- Không có permissions
- Path là flat key — không có thư mục thật trong cây

---

## DevFS — Device Filesystem

`/dev` chứa các "file" đặc biệt đại diện cho hardware và pseudo-devices.

| File | Mô tả |
|------|-------|
| `/dev/null` | Ghi → bỏ qua. Đọc → EOF |
| `/dev/zero` | Ghi → bỏ qua. Đọc → vô hạn byte 0x00 |
| `/dev/serial` | Ghi → UART COM1. Đọc → (not implemented) |

```rust
impl File for NullFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)  // EOF ngay lập tức
    }
    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        Ok(buf.len())  // bỏ qua, nhưng báo đã ghi hết
    }
}
```

Dùng trường hợp thực tế:
- `cat /dev/null` → không có output
- `cat /dev/zero | head -c 10` → 10 null bytes
- `echo "log" > /dev/serial` → ghi ra terminal (khi dùng -serial stdio)

---

## initramfs — Filesystem Khởi Động

### Vấn đề

Kernel cần có files ngay khi boot (trước khi mount disk). Làm sao có `/bin/sh` để chạy init script?

### Giải pháp: CPIO Archive

initramfs là CPIO archive nhúng vào trong kernel binary. Khi boot, kernel giải nén vào RamFS.

Linux làm tương tự — initramfs (initrd) chứa busybox và các tool cần thiết để setup boot environment trước khi mount root filesystem thật.

### CPIO newc Format

```
Mỗi entry trong CPIO archive:
┌──────────────────────────────────────┐
│ Header (110 bytes):                  │
│   Magic: "070701"                    │
│   Inode, Mode, UID, GID, ...         │
│   Filesize, Namesize                 │
├──────────────────────────────────────┤
│ Filename (Namesize bytes, null-term) │
│   (padding to 4-byte boundary)       │
├──────────────────────────────────────┤
│ File data (Filesize bytes)           │
│   (padding to 4-byte boundary)       │
└──────────────────────────────────────┘

End marker: filename = "TRAILER!!!"
```

### CpioBuilder trong MyKernel

```rust
let mut builder = CpioBuilder::new();
builder.add_dir("/bin");
builder.add_dir("/etc");
builder.add_file("/etc/hostname", b"mykernel\n");
builder.add_file("/etc/motd", b"Welcome to MyKernel!\n");
builder.add_file("/README", b"MyKernel initramfs\n");
let archive = builder.build();
```

### load_into_ramfs()

```rust
pub fn load_into_ramfs(cpio: &[u8], ramfs: &RamFs) {
    for entry in parse_cpio(cpio) {
        match entry.file_type {
            Directory => ramfs.mkdir(&entry.path),
            Regular   => ramfs.create_with_data(&entry.path, &entry.data),
        }
    }
}
```

Sau khi load, filesystem có đầy đủ:
```
/
├── README
├── bin/
│   ├── hello    ("#!/bin/sh\necho Hello!")
│   └── init     ("#!/bin/sh\necho Kernel booted!")
├── etc/
│   ├── hostname ("mykernel")
│   ├── motd     ("Welcome to MyKernel!")
│   ├── os-release
│   └── shells
└── tmp/, proc/, usr/...
```

---

## FAT32 — Filesystem trên Disk

FAT32 là format của phần lớn USB drives và thẻ nhớ. MyKernel đọc được FAT32 image từ virtio-blk device.

### Cấu trúc FAT32

```
Sector 0: Boot Sector (BPB)
  - Bytes per sector (thường 512)
  - Sectors per cluster (thường 8 = 4KB cluster)
  - Number of reserved sectors
  - Number of FATs (thường 2)
  - FAT size in sectors
  - Root cluster number

Sectors 1 - N: Reserved sectors + FAT1 + FAT2

Data area: clusters từ cluster 2 trở đi
  - Cluster 2: root directory (thường)
  - Cluster 3+: files và directories
```

### Cluster và Cluster Chain

FAT32 cấp phát storage theo **cluster** (nhóm sectors). File được chia thành các clusters liên kết:

```
File "HELLO.TXT" (3000 bytes):
  Cluster 5 → Cluster 8 → Cluster 12 → END_OF_CHAIN

FAT Table:
  FAT[5]  = 8          (next cluster là 8)
  FAT[8]  = 12         (next cluster là 12)
  FAT[12] = 0x0FFFFFF8 (end of chain)
```

Để đọc file: start cluster → đọc FAT để follow chain → đọc data từ mỗi cluster.

### Directory Entry

FAT32 có 2 loại directory entry:

**8.3 Short Name** (32 bytes):
```
Name[8]     : "HELLO   "  (padded với space)
Extension[3]: "TXT"
Attributes  : 0x20 (archive)
ClusterHigh : high 16 bits của start cluster
ClusterLow  : low 16 bits
FileSize    : 3000
```

**LFN (Long Filename)** — cho tên dài hơn 8.3:
```
Sequence number (với bit 6 = last entry)
Unicode characters (13 per entry)
Attribute = 0x0F (LFN marker)
```

LFN entries đứng trước short name entry, đọc ngược từ dưới lên.

### Tại Sao Cần Đọc FAT32?

- **Persistence**: data không mất khi reboot (khác RamFS)
- **Interoperability**: tạo disk image trên host, mount vào kernel
- **Real use case**: boot Linux với disk image, đọc files từ USB

MyKernel mount FAT32 vào `/mnt` khi có virtio-blk device. Sau đó `ls /mnt`, `cat /mnt/file.txt` hoạt động bình thường qua VFS.

---

## File Descriptor Table

Mỗi process có một bảng file descriptors (FdTable):

```
FdTable:
  fd 0 → stdin  (/dev/serial hoặc keyboard)
  fd 1 → stdout (/dev/serial hoặc VGA)
  fd 2 → stderr (/dev/serial)
  fd 3 → (sau khi open file)
  ...
  fd 99: (giới hạn cuối cùng cho files)
  fd 100+ → sockets (xem network stack)
```

```rust
struct FdTable {
    entries: Vec<Option<Arc<Mutex<dyn File>>>>,
}
```

Khi `open("/etc/hostname")`:
1. VFS tìm file trong RamFS
2. Tạo file handle `Arc<Mutex<RamFile>>`
3. Add vào FdTable, trả về index (fd = 3)
4. Syscall trả `fd = 3` cho user

Khi `read(3, buf, 100)`:
1. Lấy entry 3 từ FdTable
2. Gọi `file.lock().read(buf)`
3. Trả về số bytes đã đọc

Khi `close(3)`:
1. Remove entry 3 từ FdTable
2. `Arc` count giảm → nếu count = 0, file handle bị drop

---

## Error Handling

```rust
pub enum FsError {
    NotFound,          // file/dir không tồn tại
    FileExists,        // tạo file đã tồn tại
    NotADirectory,     // dùng directory operation trên file
    NotAFile,          // dùng file operation trên directory
    PermissionDenied,  // không có quyền
    EndOfFile,         // đọc hết file (EOF)
    IoError,           // lỗi I/O phần cứng
}
```

Rust's `Result<T, FsError>` buộc caller xử lý mọi error — không thể bỏ qua như `errno` trong C.

Shell hiển thị lỗi thân thiện:
```
kernel> cat /nonexistent
cat: /nonexistent: NotFound

kernel> cat /etc
cat: /etc: NotAFile
```
