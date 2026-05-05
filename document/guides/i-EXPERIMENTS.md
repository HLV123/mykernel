# Thí Nghiệm Thực Hành

> Các thí nghiệm nhỏ để tự tay sửa code và quan sát kết quả — cách học hiệu quả nhất là làm.
> Mỗi thí nghiệm có mức độ khó từ ⭐ (dễ) đến ⭐⭐⭐ (trung bình). 
> Chỉ thực hiện trên bản sao để tránh lỡ dại phải kéo repo lại từ đầu

---

## Trước khi bắt đầu

Mỗi lần sửa xong, build và chạy lại:
```powershell
cargo bootimage
qemu-system-x86_64 -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin -serial stdio -no-reboot
```

Nếu có lỗi compile, đọc kỹ error message — Rust compiler rất chi tiết và thường gợi ý cách fix.

Để hoàn tác thí nghiệm, dùng `git checkout src/filename.rs` (nếu đã init git), hoặc copy lại từ file backup.

---

## Thí Nghiệm 1: Thay đổi màu chữ VGA ⭐

**Mục tiêu:** Hiểu VGA text mode và color encoding.

**File cần sửa:** `src/vga_buffer.rs`

**Tìm đoạn:**
```rust
pub fn write_byte(&mut self, byte: u8) {
    // ...
    self.buffer.chars[row][self.column_position].write(ScreenChar {
        ascii_character: byte,
        color_code: self.color_code,
    });
```

**Tìm chỗ khởi tạo default color:**
```rust
lazy_static! {
    pub static ref WRITER: Mutex<Writer> = Mutex::new(Writer {
        column_position: 0,
        color_code: ColorCode::new(Color::Yellow, Color::Black),
        // ...
    });
}
```

**Thử nghiệm:**
- Đổi `Color::Yellow` thành `Color::Green` → chữ xanh
- Đổi `Color::Black` thành `Color::Blue` → nền xanh
- Thử `Color::White` với `Color::Red` → chữ trắng nền đỏ

**Các màu có thể dùng:**
```rust
pub enum Color {
    Black, Blue, Green, Cyan, Red, Magenta, Brown,
    LightGray, DarkGray, LightBlue, LightGreen, LightCyan,
    LightRed, Pink, Yellow, White,
}
```

**Quan sát:** Chạy kernel, toàn bộ chữ đổi màu. Màu VGA được mã hóa trong 1 byte: 4 bits cho foreground, 4 bits cho background.

---

## Thí Nghiệm 2: Thêm lệnh shell mới ⭐

**Mục tiêu:** Hiểu cách shell dispatch commands và thêm feature mới.

**File cần sửa:** `src/shell.rs`

**Thêm lệnh `hello` in ra "Hello, World!":**

**Bước 1:** Tìm `match cmd {` và thêm case mới:
```rust
match cmd {
    "help"     => cmd_help(),
    "uname"    => cmd_uname(),
    // ... các lệnh khác ...
    "hello"    => cmd_hello(arg1),  // ← thêm dòng này
    other      => println!("unknown command: '{}'.  Type 'help'.", other),
}
```

**Bước 2:** Thêm function `cmd_hello` ở cuối file:
```rust
/// Print a greeting message.
fn cmd_hello(name: &str) {
    if name.is_empty() {
        println!("Hello, World!");
    } else {
        println!("Hello, {}!", name);
    }
}
```

**Bước 3:** Thêm vào `cmd_help()`:
```rust
fn cmd_help() {
    println!("Available commands:");
    // ...
    println!("  hello [name]      -- print a greeting");
    // ...
}
```

**Test:**
```
kernel> hello
Hello, World!

kernel> hello MyKernel
Hello, MyKernel!
```

**Mở rộng:** Thêm lệnh `date` in ra tick count dạng timestamp, hay lệnh `whoami` in ra "root".

---

## Thí Nghiệm 3: Thêm file vào initramfs ⭐

**Mục tiêu:** Hiểu cách initramfs hoạt động — files được nhúng vào kernel binary.

**File cần sửa:** `src/fs/initramfs.rs`

**Tìm function `create_default_initramfs()`:**
```rust
pub fn create_default_initramfs() -> Vec<u8> {
    let mut builder = CpioBuilder::new();

    // Directories
    builder.add_dir("/bin");
    builder.add_dir("/etc");
    // ...

    // Files
    builder.add_file("/etc/hostname", b"mykernel\n");
    builder.add_file("/etc/motd",
        b"Welcome to MyKernel!\nBuilt with Rust. Phase 15: initramfs\n");
    // ...
```

**Thêm file mới:**
```rust
// Thêm file của bạn
builder.add_file("/etc/myconfig", b"my_setting=hello\nversion=1.0\n");
builder.add_file("/bin/greet", b"#!/bin/sh\necho 'Greetings from MyKernel!'\n");
```

**Thêm thư mục mới:**
```rust
builder.add_dir("/home");
builder.add_dir("/home/user");
builder.add_file("/home/user/welcome.txt", b"Welcome to your home directory!\n");
```

**Test:**
```
kernel> ls /etc
  -    9  hostname
  -   58  motd
  -   30  myconfig    ← file mới!

kernel> cat /etc/myconfig
my_setting=hello
version=1.0

kernel> ls /home/user
  -   32  welcome.txt
```

**Quan sát:** Files này là một phần của kernel binary — không cần disk để có chúng.

---

## Thí Nghiệm 4: Thay đổi boot banner ⭐

**Mục tiêu:** Hiểu boot sequence và cách customize output.

**File cần sửa:** `src/main.rs`

**Tìm function `print_banner()` và sửa:**
```rust
fn print_banner() {
    println!("");
    println!("  __  __       _  __                    _");
    // ... ASCII art ...

    println!("  Bare-metal OS kernel  |  Rust  |  x86_64");
    println!("");
```

**Thêm thông tin custom:**
```rust
    println!("  Bare-metal OS kernel  |  Rust  |  x86_64");
    println!("  Built by: [Your Name Here]");  // ← thêm dòng này
    println!("  Version: 1.0.0-alpha");
    println!("");
```

**Hoặc thay ASCII art bằng text đơn giản:**
```rust
fn print_banner() {
    println!("");
    println!("╔════════════════════════════╗");
    println!("║      MY CUSTOM KERNEL      ║");
    println!("║    Written in Rust 🦀      ║");
    println!("╚════════════════════════════╝");
    println!("");
```

---

## Thí Nghiệm 5: Thay đổi security score ⭐⭐

**Mục tiêu:** Hiểu security subsystem và cách tính điểm.

**File cần sửa:** `src/security.rs`

**Tìm `SecurityAudit.score()`:**
```rust
impl SecurityAudit {
    pub fn score(&self) -> u32 {
        let mut score = 0u32;
        if self.smep_enabled    { score += 20; }
        if self.smap_enabled    { score += 20; }
        if self.nx_enabled      { score += 15; }
        if self.canary_set      { score += 15; }
        if self.kaslr_active    { score += 15; }
        if self.hardened_policy { score += 15; }
        score
    }
}
```

**Thử nghiệm 1:** Tắt hardened policy và xem score giảm:
Tìm `set_hardened_policy()` và comment dòng gọi nó trong `init()`:
```rust
pub fn init() {
    init_entropy();
    init_stack_canary();
    init_kaslr();
    enable_cpu_security_features();
    // set_hardened_policy();  // ← comment out
}
```

Chạy `security` trong shell → score giảm 15.

**Thử nghiệm 2:** Thêm tiêu chí mới vào scoring:
```rust
pub struct SecurityAudit {
    // ... fields cũ ...
    pub has_secret_feature: bool,  // thêm field mới
}

impl SecurityAudit {
    pub fn score(&self) -> u32 {
        let mut score = /* ... */;
        if self.has_secret_feature { score += 10; }  // +10 điểm
        score
    }
}
```

---

## Thí Nghiệm 6: Tăng heap size và test OOM ⭐⭐

**Mục tiêu:** Hiểu heap allocator và memory limits.

**File cần sửa:** `src/allocator.rs`

**Giảm heap xuống rất nhỏ:**
```rust
pub const HEAP_SIZE: usize = 10 * 1024;  // 10KB (rất nhỏ)
```

**Chạy kernel và thử:**
```
kernel> write /tmp/a.txt AAAAAAAAAAAAA...  (string dài)
```

→ Sẽ thấy KERNEL PANIC: memory allocation failed khi heap cạn.

**Tăng heap lên:**
```rust
pub const HEAP_SIZE: usize = 1024 * 1024;  // 1MB
```

→ Có thể allocate nhiều hơn.

**Lưu ý:** Heap quá lớn thì page mapping init sẽ cần nhiều frames hơn. Nếu OOM trong `init_heap`, tăng frame allocator limit.

---

## Thí Nghiệm 7: Thêm thông tin vào `uname` ⭐⭐

**Mục tiêu:** Đọc thông tin từ CPUID và hiển thị.

**File cần sửa:** `src/shell.rs`

**Tìm `cmd_uname()` và thêm thông tin:**
```rust
fn cmd_uname() {
    println!("Kernel:  MyKernel");
    println!("Release: 1.0.0");
    println!("Arch:    x86_64");
    println!("Build:   Rust bare-metal (no_std)");

    let brand = cpu_brand_string();
    println!("CPU:     {}", brand.trim());

    // Thêm thông tin APIC
    let bsp_id = crate::apic::lapic_id();
    println!("BSP APIC ID: {}", bsp_id);

    // Thêm uptime
    let ticks = crate::sync::get_ticks();
    println!("Uptime:  {} ticks ({} ms)", ticks, ticks * 10);

    // Thêm heap info
    println!("Heap:    {} KB", crate::allocator::HEAP_SIZE / 1024);
}
```

---

## Thí Nghiệm 8: Chạy với nhiều CPUs ⭐⭐

**Mục tiêu:** Hiểu SMP và quan sát multi-core boot.

**Thêm `-smp 4` vào lệnh QEMU:**
```powershell
qemu-system-x86_64 `
  -drive format=raw,file=target\x86_64-mykernel\debug\bootimage-mykernel.bin `
  -smp 4 `
  -serial stdio -no-reboot
```

**Quan sát output boot:**
```
[apic] Local APIC ID=0 Version=0x14 MaxLVT=5
[smp] Found 4 processors in MADT
[smp] Booting AP 1...
[smp] Booting AP 2...
[smp] Booting AP 3...
[smp] All APs online (3 additional)
```

**Chạy `cpu` trong shell:**
```
kernel> cpu
CPU topology:
  BSP APIC ID: 0
  Total CPUs:  4
  Online CPUs: 4
```

**Thử `-smp 1`, `-smp 2`, `-smp 8`** và quan sát sự thay đổi.

---

## Thí Nghiệm 9: Thêm lệnh `hexdump` ⭐⭐

**Mục tiêu:** Luyện tập viết shell command với file I/O.

**File cần sửa:** `src/shell.rs`

**Thêm lệnh:**
```rust
// Trong match cmd:
"hexdump" => cmd_hexdump(arg1),

// Function mới:
fn cmd_hexdump(path: &str) {
    if path.is_empty() {
        println!("usage: hexdump <file>");
        return;
    }
    match crate::fs::read_file(path) {
        Ok(data) => {
            println!("File: {} ({} bytes)", path, data.len());
            println!("");
            for (i, chunk) in data.chunks(16).enumerate() {
                // Offset
                print!("{:08x}  ", i * 16);
                // Hex
                for b in chunk {
                    print!("{:02x} ", b);
                }
                // Padding
                for _ in chunk.len()..16 { print!("   "); }
                // ASCII
                print!(" |");
                for &b in chunk {
                    if b.is_ascii_graphic() {
                        print!("{}", b as char);
                    } else {
                        print!(".");
                    }
                }
                println!("|");
            }
        }
        Err(e) => println!("hexdump: {}: {:?}", path, e),
    }
}
```

**Test:**
```
kernel> hexdump /etc/hostname
File: /etc/hostname (9 bytes)

00000000  6d 79 6b 65 72 6e 65 6c  0a 00 00 00 00 00 00 00  |mykernel........|
```

---

## Thí Nghiệm 10: Thêm /dev/random ⭐⭐⭐

**Mục tiêu:** Thêm device file mới vào DevFS.

**File cần sửa:** `src/fs/devfs.rs`

**Bước 1:** Tìm `DevFs` và xem cách `/dev/zero` được implement.

**Bước 2:** Thêm `RandomFile`:
```rust
struct RandomFile;

impl File for RandomFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        // Dùng security::fill_random để fill buffer
        crate::security::fill_random(buf);
        Ok(buf.len())
    }
    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        // Bỏ qua (đây là entropy sink trong Linux thật)
        Ok(buf.len())
    }
    fn stat(&self) -> FsResult<Stat> { /* ... */ }
    fn seek(&mut self, _: i64, _: SeekWhence) -> FsResult<u64> { Ok(0) }
}
```

**Bước 3:** Đăng ký trong DevFs `open()`:
```rust
fn open(&self, path: &str, _flags: OpenFlags) -> FsResult<Arc<Mutex<dyn File>>> {
    match path {
        "/null"   => Ok(Arc::new(Mutex::new(NullFile))),
        "/zero"   => Ok(Arc::new(Mutex::new(ZeroFile))),
        "/serial" => Ok(Arc::new(Mutex::new(SerialFile))),
        "/random" => Ok(Arc::new(Mutex::new(RandomFile))),  // ← thêm
        _ => Err(FsError::NotFound),
    }
}
```

**Test:**
```
kernel> cat /dev/random
(16 bytes random mỗi lần đọc)
```

---

## Ghi Chú Khi Gặp Lỗi

**Lỗi compile phổ biến:**

```
error[E0425]: cannot find function `xxx` in module `yyy`
→ Kiểm tra tên function đúng chưa, có pub không, có use đúng không

error[E0308]: mismatched types
→ Kiểm tra kiểu trả về, có thể cần .into(), as, hoặc ?

error: unused variable: `x`
→ Đổi thành `_x` hoặc thêm `#[allow(unused)]`
```

**Kernel panic lúc chạy:**

```
KERNEL PANIC: memory allocation of X bytes failed
→ Tăng HEAP_SIZE trong allocator.rs

KERNEL PANIC: panicked at ...
→ Đọc file và dòng trong message, thêm serial_println! để debug
```

**QEMU không start:**
```
Could not open 'bootimage-mykernel.bin'
→ Chạy cargo bootimage trước
```
