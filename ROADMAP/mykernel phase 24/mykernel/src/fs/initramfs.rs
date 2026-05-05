/// initramfs — Initial RAM Filesystem
///
/// Parse CPIO "newc" format archive và populate VFS rootfs.
///
/// CPIO newc format:
///   - Magic: "070701" (6 bytes)
///   - Header: 13 fields × 8 hex digits = 104 bytes
///   - Filename: c_namesize bytes (null-terminated)
///   - Padding to 4-byte alignment
///   - File data: c_filesize bytes
///   - Padding to 4-byte alignment
///   - Special entry "TRAILER!!!" marks end
///
/// Dùng để boot: kernel embed một CPIO archive trong binary,
/// giải nén vào ramfs lúc boot → có files ngay từ đầu.

use alloc::string::String;
use alloc::vec::Vec;

use super::ramfs::RamFs;
use super::vfs;

// ---------------------------------------------------------------------------
// CPIO newc header (110 bytes total)
// ---------------------------------------------------------------------------

// Magic string cho newc format
const CPIO_NEWC_MAGIC: &[u8; 6] = b"070701";
const CPIO_TRAILER: &str = "TRAILER!!!";

/// Parse một số hex 8 chữ số từ CPIO header
fn parse_hex8(bytes: &[u8]) -> u64 {
    let mut val = 0u64;
    for &b in &bytes[..8] {
        val <<= 4;
        val |= match b {
            b'0'..=b'9' => (b - b'0') as u64,
            b'a'..=b'f' => (b - b'a' + 10) as u64,
            b'A'..=b'F' => (b - b'A' + 10) as u64,
            _ => 0,
        };
    }
    val
}

/// Round up to 4-byte alignment
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

// ---------------------------------------------------------------------------
// CPIO Entry
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct CpioEntry<'a> {
    pub name: &'a str,
    pub mode: u32,
    pub size: u64,
    pub data: &'a [u8],
}

/// Iterator qua các entries trong CPIO archive
pub struct CpioIterator<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CpioIterator<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        CpioIterator { data, pos: 0 }
    }
}

impl<'a> Iterator for CpioIterator<'a> {
    type Item = CpioEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let data = self.data;
        let pos = self.pos;

        // Cần ít nhất 110 bytes cho header
        if pos + 110 > data.len() {
            return None;
        }

        // Check magic
        if &data[pos..pos+6] != CPIO_NEWC_MAGIC {
            crate::serial_println!("[cpio] Invalid magic at offset {:#x}", pos);
            return None;
        }

        // Parse header fields (mỗi field 8 hex chars)
        // c_ino      [6..14]
        let c_mode     = parse_hex8(&data[pos+14..]) as u32;  // [14..22]
        // c_uid      [22..30]
        // c_gid      [30..38]
        // c_nlink    [38..46]
        // c_mtime    [46..54]
        let c_filesize = parse_hex8(&data[pos+54..]) as usize; // [54..62]
        // c_devmajor [62..70]
        // c_devminor [70..78]
        // c_rdevmajor[78..86]
        // c_rdevminor[86..94]
        let c_namesize = parse_hex8(&data[pos+94..]) as usize; // [94..102]
        // c_check    [102..110]

        // Filename starts at offset 110
        let name_start = pos + 110;
        let name_end = name_start + c_namesize;
        if name_end > data.len() { return None; }

        // Filename is null-terminated
        let name_bytes = &data[name_start..name_end - 1]; // strip null
        let name = core::str::from_utf8(name_bytes).unwrap_or("?");

        // Check for trailer
        if name == CPIO_TRAILER {
            return None;
        }

        // Data starts after header + name + padding
        let data_start = align4(name_end);
        let data_end = data_start + c_filesize;
        if data_end > data.len() { return None; }

        let file_data = &data[data_start..data_end];

        // Advance pos to next entry
        self.pos = align4(data_end);

        Some(CpioEntry {
            name,
            mode: c_mode,
            size: c_filesize as u64,
            data: file_data,
        })
    }
}

// ---------------------------------------------------------------------------
// Load initramfs vào VFS
// ---------------------------------------------------------------------------

/// Parse CPIO archive và populate ramfs với các files/dirs
pub fn load_into_ramfs(cpio_data: &[u8], ramfs: &RamFs) {
    crate::serial_println!("[initramfs] Loading {} bytes", cpio_data.len());

    let mut file_count = 0;
    let mut dir_count = 0;

    for entry in CpioIterator::new(cpio_data) {
        let name = entry.name;
        let mode = entry.mode;

        // Skip "." (current dir)
        if name == "." { continue; }

        // Normalize path: prepend "/" if needed
        let path = if name.starts_with('/') {
            String::from(name)
        } else {
            let mut s = String::from("/");
            s.push_str(name);
            s
        };

        // mode & 0o170000 determines file type:
        // 0o040000 = directory
        // 0o100000 = regular file
        // 0o120000 = symlink
        let is_dir  = (mode & 0o170000) == 0o040000;
        let is_file = (mode & 0o170000) == 0o100000;

        if is_dir {
            // Create directory (ignore errors — may already exist)
            let _ = vfs::mkdir(&path);
            dir_count += 1;
            crate::serial_println!("[initramfs] mkdir {}", path);
        } else if is_file {
            ramfs.write_file(&path, entry.data);
            file_count += 1;
        }
        // Symlinks: skip for now
    }

    crate::serial_println!(
        "[initramfs] Loaded: {} files, {} dirs",
        file_count, dir_count
    );
}

// ---------------------------------------------------------------------------
// Build a CPIO archive in memory (for testing / embedding)
// ---------------------------------------------------------------------------

/// Builder để tạo CPIO newc archive trong memory
pub struct CpioBuilder {
    data: Vec<u8>,
}

impl CpioBuilder {
    pub fn new() -> Self {
        CpioBuilder { data: Vec::new() }
    }

    /// Thêm một file vào archive
    pub fn add_file(mut self, path: &str, content: &[u8]) -> Self {
        self.write_entry(path, 0o100644, content);
        self
    }

    /// Thêm một directory
    pub fn add_dir(mut self, path: &str) -> Self {
        self.write_entry(path, 0o040755, &[]);
        self
    }

    fn write_entry(&mut self, name: &str, mode: u32, data: &[u8]) {
        static INODE: core::sync::atomic::AtomicU64 =
            core::sync::atomic::AtomicU64::new(1);
        let ino = INODE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        let name_with_null = {
            let mut v: Vec<u8> = Vec::new();
            v.extend_from_slice(name.as_bytes());
            v.push(0); // null terminator
            v
        };
        let namesize = name_with_null.len();
        let filesize = data.len();

        // Write 110-byte header
        self.data.extend_from_slice(CPIO_NEWC_MAGIC);
        self.write_hex8(ino);          // c_ino
        self.write_hex8(mode as u64);  // c_mode
        self.write_hex8(0);            // c_uid
        self.write_hex8(0);            // c_gid
        self.write_hex8(1);            // c_nlink
        self.write_hex8(0);            // c_mtime
        self.write_hex8(filesize as u64); // c_filesize
        self.write_hex8(0);            // c_devmajor
        self.write_hex8(0);            // c_devminor
        self.write_hex8(0);            // c_rdevmajor
        self.write_hex8(0);            // c_rdevminor
        self.write_hex8(namesize as u64); // c_namesize
        self.write_hex8(0);            // c_check

        // Write filename
        self.data.extend_from_slice(&name_with_null);
        // Pad to 4-byte alignment (header=110, name starts at 110)
        let name_end = 110 + namesize;
        let pad = align4(name_end) - name_end;
        for _ in 0..pad { self.data.push(0); }

        // Write file data
        self.data.extend_from_slice(data);
        // Pad data to 4-byte alignment
        let data_end_pos = self.data.len();
        let data_pad = align4(filesize) - filesize;
        for _ in 0..data_pad { self.data.push(0); }
    }

    fn write_hex8(&mut self, val: u64) {
        let s = alloc::format!("{:08X}", val & 0xFFFFFFFF);
        self.data.extend_from_slice(s.as_bytes());
    }

    /// Finalize archive với TRAILER entry
    pub fn build(mut self) -> Vec<u8> {
        self.write_entry(CPIO_TRAILER, 0, &[]);
        self.data
    }
}

// ---------------------------------------------------------------------------
// Embedded initramfs (minimal default)
// ---------------------------------------------------------------------------

/// Tạo một initramfs tối thiểu để test
/// Trong production: cpio archive sẽ được link vào kernel binary
pub fn create_default_initramfs() -> Vec<u8> {
    CpioBuilder::new()
        .add_dir(".")
        .add_dir("bin")
        .add_dir("etc")
        .add_dir("tmp")
        .add_dir("proc")
        .add_dir("dev")
        .add_dir("usr")
        .add_dir("usr/bin")
        .add_file("etc/hostname",  b"mykernel\n")
        .add_file("etc/os-release", b"NAME=MyKernel\nVERSION=0.1.0\nID=mykernel\n")
        .add_file("etc/motd",      b"Welcome to MyKernel!\nBuilt with Rust. Phase 15: initramfs\n")
        .add_file("etc/shells",    b"/bin/sh\n")
        .add_file("bin/hello",     b"#!/bin/sh\necho Hello from initramfs!\n")
        .add_file("bin/init",      b"#!/bin/sh\necho Init started!\nmount -t proc proc /proc\nexec /bin/sh\n")
        .add_file("README",        b"MyKernel initramfs\nLoaded via CPIO newc format.\n")
        .build()
}
