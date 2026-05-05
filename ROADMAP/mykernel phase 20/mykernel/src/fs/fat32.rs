/// FAT32 Filesystem
///
/// FAT32 layout on disk:
///   Sector 0:        Boot Sector (BPB - BIOS Parameter Block)
///   Sector 1..N:     Reserved sectors (FAT32 info sector at 1)
///   Sector R..R+S*2: FAT tables (2 copies)
///   Sector D..:      Data region (clusters 2+)
///
/// Key concepts:
///   - Everything measured in clusters (e.g. 8 sectors = 4096 bytes)
///   - FAT table: array of u32, each entry points to next cluster or EOF
///   - Directory entries: 32 bytes each, stored in data region
///   - Long filename entries (LFN): special dir entries before normal entry

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use super::vfs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, OpenFlags, SeekWhence, Stat,
};

// ---------------------------------------------------------------------------
// FAT32 on-disk structures
// ---------------------------------------------------------------------------

/// BPB (BIOS Parameter Block) — Boot Sector
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct Fat32Bpb {
    pub jump_boot:       [u8; 3],
    pub oem_name:        [u8; 8],
    pub bytes_per_sector:    u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors:    u16,
    pub num_fats:            u8,
    pub root_entry_count:    u16,  // 0 for FAT32
    pub total_sectors_16:    u16,  // 0 for FAT32
    pub media_type:          u8,
    pub fat_size_16:         u16,  // 0 for FAT32
    pub sectors_per_track:   u16,
    pub num_heads:           u16,
    pub hidden_sectors:      u32,
    pub total_sectors_32:    u32,
    // FAT32 extended BPB
    pub fat_size_32:         u32,
    pub ext_flags:           u16,
    pub fs_version:          u16,
    pub root_cluster:        u32,  // Usually 2
    pub fs_info:             u16,
    pub backup_boot_sector:  u16,
    pub reserved:            [u8; 12],
    pub drive_number:        u8,
    pub reserved1:           u8,
    pub boot_signature:      u8,
    pub volume_id:           u32,
    pub volume_label:        [u8; 11],
    pub fs_type:             [u8; 8],  // "FAT32   "
}

/// FAT32 Directory Entry (32 bytes)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct Fat32DirEntry {
    pub name:       [u8; 8],   // 8.3 short name
    pub ext:        [u8; 3],
    pub attr:       u8,        // File attributes
    pub nt_res:     u8,
    pub crt_time_tenth: u8,
    pub crt_time:   u16,
    pub crt_date:   u16,
    pub lst_acc_date: u16,
    pub fst_clus_hi: u16,     // High 16 bits of first cluster
    pub wrt_time:   u16,
    pub wrt_date:   u16,
    pub fst_clus_lo: u16,     // Low 16 bits of first cluster
    pub file_size:  u32,
}

/// FAT32 Long Filename Entry
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct Fat32LfnEntry {
    pub order:      u8,
    pub name1:      [u16; 5],
    pub attr:       u8,   // Always 0x0F
    pub type_:      u8,
    pub checksum:   u8,
    pub name2:      [u16; 6],
    pub fst_clus:   u16,  // Always 0
    pub name3:      [u16; 2],
}

// File attribute bits
pub const ATTR_READ_ONLY: u8 = 0x01;
pub const ATTR_HIDDEN:    u8 = 0x02;
pub const ATTR_SYSTEM:    u8 = 0x04;
pub const ATTR_VOLUME_ID: u8 = 0x08;
pub const ATTR_DIRECTORY: u8 = 0x10;
pub const ATTR_ARCHIVE:   u8 = 0x20;
pub const ATTR_LFN:       u8 = 0x0F; // Long filename entry

// FAT32 special cluster values
pub const FAT32_EOC: u32 = 0x0FFFFFF8; // End of chain
pub const FAT32_FREE: u32 = 0x00000000;

// ---------------------------------------------------------------------------
// Block device abstraction for FAT32
// ---------------------------------------------------------------------------

pub trait BlockDevice: Send + Sync {
    fn read_sector(&self, sector: u64, buf: &mut [u8; 512]) -> FsResult<()>;
    fn write_sector(&self, sector: u64, buf: &[u8; 512]) -> FsResult<()>;
}

// ---------------------------------------------------------------------------
// FAT32 Filesystem
// ---------------------------------------------------------------------------

pub struct Fat32Fs {
    device: Arc<dyn BlockDevice>,
    bpb: Fat32Bpb,
    fat_start: u64,    // First sector of FAT
    data_start: u64,   // First sector of data region
    sectors_per_cluster: u64,
    bytes_per_cluster: u64,
}

impl Fat32Fs {
    /// Mount FAT32 filesystem từ block device
    pub fn new(device: Arc<dyn BlockDevice>) -> FsResult<Arc<Self>> {
        // Read boot sector
        let mut sector = [0u8; 512];
        device.read_sector(0, &mut sector).map_err(|_| FsError::Io)?;

        // Validate FAT32 signature
        if sector[510] != 0x55 || sector[511] != 0xAA {
            crate::serial_println!("[fat32] Invalid boot signature");
            return Err(FsError::InvalidArgument);
        }

        let bpb = unsafe { *(sector.as_ptr() as *const Fat32Bpb) };

        // Validate FAT32
        let bytes_per_sector = { bpb.bytes_per_sector } as u64;
        let sectors_per_cluster = { bpb.sectors_per_cluster } as u64;
        let reserved = { bpb.reserved_sectors } as u64;
        let num_fats = { bpb.num_fats } as u64;
        let fat_size = { bpb.fat_size_32 } as u64;

        if bytes_per_sector != 512 {
            crate::serial_println!("[fat32] Unsupported sector size: {}", bytes_per_sector);
            return Err(FsError::InvalidArgument);
        }

        let fat_start = reserved;
        let data_start = reserved + num_fats * fat_size;
        let bytes_per_cluster = sectors_per_cluster * 512;

        crate::serial_println!(
            "[fat32] Mounted: fat_start={} data_start={} spc={} bpc={}",
            fat_start, data_start, sectors_per_cluster, bytes_per_cluster
        );

        Ok(Arc::new(Fat32Fs {
            device,
            bpb,
            fat_start,
            data_start,
            sectors_per_cluster,
            bytes_per_cluster,
        }))
    }

    /// Convert cluster number to sector number
    fn cluster_to_sector(&self, cluster: u32) -> u64 {
        self.data_start + (cluster as u64 - 2) * self.sectors_per_cluster
    }

    /// Read next cluster from FAT
    fn fat_next_cluster(&self, cluster: u32) -> FsResult<Option<u32>> {
        let fat_offset = cluster as u64 * 4;
        let fat_sector = self.fat_start + fat_offset / 512;
        let byte_offset = (fat_offset % 512) as usize;

        let mut buf = [0u8; 512];
        self.device.read_sector(fat_sector, &mut buf)
            .map_err(|_| FsError::Io)?;

        let next = u32::from_le_bytes([
            buf[byte_offset],
            buf[byte_offset + 1],
            buf[byte_offset + 2],
            buf[byte_offset + 3],
        ]) & 0x0FFFFFFF;

        if next >= 0x0FFFFFF8 {
            Ok(None) // End of chain
        } else if next == 0 {
            Ok(None) // Free cluster (shouldn't happen in valid chain)
        } else {
            Ok(Some(next))
        }
    }

    /// Read all sectors of a cluster chain into a Vec
    fn read_cluster_chain(&self, start_cluster: u32) -> FsResult<Vec<u8>> {
        let mut data = Vec::new();
        let mut cluster = start_cluster;

        loop {
            let sector = self.cluster_to_sector(cluster);
            for i in 0..self.sectors_per_cluster {
                let mut buf = [0u8; 512];
                self.device.read_sector(sector + i, &mut buf)
                    .map_err(|_| FsError::Io)?;
                data.extend_from_slice(&buf);
            }

            match self.fat_next_cluster(cluster)? {
                Some(next) => cluster = next,
                None => break,
            }
        }

        Ok(data)
    }

    /// Parse directory entries từ raw data
    fn parse_directory(&self, data: &[u8]) -> Vec<Fat32FileInfo> {
        let mut entries = Vec::new();
        let mut lfn_buf: Vec<u16> = Vec::new();
        let mut lfn_order = 0u8;

        let num_entries = data.len() / 32;
        for i in 0..num_entries {
            let offset = i * 32;
            if offset + 32 > data.len() { break; }

            let entry_data = &data[offset..offset + 32];
            let first_byte = entry_data[0];

            // End of directory
            if first_byte == 0x00 { break; }
            // Deleted entry
            if first_byte == 0xE5 {
                lfn_buf.clear();
                continue;
            }

            let attr = entry_data[11];

            // LFN entry
            if attr == ATTR_LFN {
                let lfn = unsafe { *(entry_data.as_ptr() as *const Fat32LfnEntry) };
                let order = { lfn.order };

                if order & 0x40 != 0 {
                    // First LFN entry (last in sequence)
                    lfn_buf.clear();
                    lfn_order = order & 0x3F;
                }

                // Collect name chars (little-endian UTF-16)
                let mut chars: Vec<u16> = Vec::new();
                let name1 = { lfn.name1 };
                let name2 = { lfn.name2 };
                let name3 = { lfn.name3 };
                for c in &name1 { if *c != 0 && *c != 0xFFFF { chars.push(*c); } }
                for c in &name2 { if *c != 0 && *c != 0xFFFF { chars.push(*c); } }
                for c in &name3 { if *c != 0 && *c != 0xFFFF { chars.push(*c); } }

                // Prepend to buffer (LFN entries come in reverse order)
                let mut new_buf = chars;
                new_buf.extend_from_slice(&lfn_buf);
                lfn_buf = new_buf;
                continue;
            }

            // Regular directory entry
            if attr & ATTR_VOLUME_ID != 0 {
                lfn_buf.clear();
                continue;
            }

            let dir_entry = unsafe { *(entry_data.as_ptr() as *const Fat32DirEntry) };
            let fst_clus_hi = { dir_entry.fst_clus_hi } as u32;
            let fst_clus_lo = { dir_entry.fst_clus_lo } as u32;
            let cluster = (fst_clus_hi << 16) | fst_clus_lo;
            let file_size = { dir_entry.file_size };
            let is_dir = attr & ATTR_DIRECTORY != 0;

            // Get filename
            let name = if !lfn_buf.is_empty() {
                // Use LFN
                String::from_utf16_lossy(&lfn_buf).trim_end_matches('\0').to_string()
            } else {
                // Use 8.3 short name
                let name_bytes = &dir_entry.name;
                let ext_bytes = &dir_entry.ext;
                let name_str = core::str::from_utf8(name_bytes).unwrap_or("")
                    .trim_end_matches(' ');
                let ext_str = core::str::from_utf8(ext_bytes).unwrap_or("")
                    .trim_end_matches(' ');
                if ext_str.is_empty() {
                    name_str.to_string()
                } else {
                    alloc::format!("{}.{}", name_str, ext_str)
                }
            };

            lfn_buf.clear();

            // Skip . and ..
            if name == "." || name == ".." { continue; }

            entries.push(Fat32FileInfo {
                name,
                cluster,
                file_size,
                is_dir,
            });
        }

        entries
    }

    /// Find entry in directory cluster chain by name
    fn find_in_dir(&self, dir_cluster: u32, name: &str) -> FsResult<Fat32FileInfo> {
        let data = self.read_cluster_chain(dir_cluster)?;
        let entries = self.parse_directory(&data);

        entries.into_iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
            .ok_or(FsError::NotFound)
    }

    /// Resolve path to (cluster, file_size, is_dir)
    fn resolve_path(&self, path: &str) -> FsResult<Fat32FileInfo> {
        let root_cluster = { self.bpb.root_cluster };
        let path = path.trim_start_matches('/');

        if path.is_empty() {
            return Ok(Fat32FileInfo {
                name: String::from("/"),
                cluster: root_cluster,
                file_size: 0,
                is_dir: true,
            });
        }

        let mut current_cluster = root_cluster;
        let mut current_info = Fat32FileInfo {
            name: String::from("/"),
            cluster: root_cluster,
            file_size: 0,
            is_dir: true,
        };

        for component in path.split('/') {
            if component.is_empty() { continue; }
            if !current_info.is_dir {
                return Err(FsError::NotADirectory);
            }
            current_info = self.find_in_dir(current_cluster, component)?;
            current_cluster = current_info.cluster;
        }

        Ok(current_info)
    }
}

#[derive(Debug, Clone)]
struct Fat32FileInfo {
    name: String,
    cluster: u32,
    file_size: u32,
    is_dir: bool,
}

// ---------------------------------------------------------------------------
// Fat32File — open file handle
// ---------------------------------------------------------------------------

pub struct Fat32File {
    data: Vec<u8>,
    pos: usize,
    size: u64,
}

impl File for Fat32File {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let available = self.data.len().saturating_sub(self.pos);
        if available == 0 { return Err(FsError::EndOfFile); }
        let to_read = buf.len().min(available);
        buf[..to_read].copy_from_slice(&self.data[self.pos..self.pos + to_read]);
        self.pos += to_read;
        Ok(to_read)
    }

    fn write(&mut self, _buf: &[u8]) -> FsResult<usize> {
        Err(FsError::PermissionDenied) // Read-only for now
    }

    fn seek(&mut self, offset: i64, whence: SeekWhence) -> FsResult<u64> {
        let len = self.data.len() as i64;
        let new_pos = match whence {
            SeekWhence::Set => offset,
            SeekWhence::Cur => self.pos as i64 + offset,
            SeekWhence::End => len + offset,
        };
        if new_pos < 0 { return Err(FsError::InvalidArgument); }
        self.pos = new_pos as usize;
        Ok(self.pos as u64)
    }

    fn stat(&self) -> FsResult<Stat> {
        Ok(Stat { file_type: FileType::RegularFile, size: self.size, inode: 0 })
    }
}

// ---------------------------------------------------------------------------
// FileSystem impl for Fat32Fs
// ---------------------------------------------------------------------------

impl FileSystem for Fat32Fs {
    fn name(&self) -> &str { "fat32" }

    fn open(&self, path: &str, _flags: OpenFlags) -> FsResult<Arc<Mutex<dyn File>>> {
        let info = self.resolve_path(path)?;
        if info.is_dir { return Err(FsError::IsADirectory); }

        let data = if info.cluster >= 2 {
            let mut d = self.read_cluster_chain(info.cluster)?;
            d.truncate(info.file_size as usize);
            d
        } else {
            Vec::new()
        };

        let size = info.file_size as u64;
        Ok(Arc::new(Mutex::new(Fat32File { data, pos: 0, size })))
    }

    fn create(&self, _path: &str) -> FsResult<Arc<Mutex<dyn File>>> {
        Err(FsError::PermissionDenied) // Read-only
    }

    fn unlink(&self, _path: &str) -> FsResult<()> {
        Err(FsError::PermissionDenied)
    }

    fn readdir(&self, path: &str) -> FsResult<Vec<DirEntry>> {
        let info = self.resolve_path(path)?;
        if !info.is_dir { return Err(FsError::NotADirectory); }

        let data = self.read_cluster_chain(info.cluster)?;
        let entries = self.parse_directory(&data);

        Ok(entries.into_iter().map(|e| DirEntry {
            name: e.name,
            file_type: if e.is_dir { FileType::Directory } else { FileType::RegularFile },
            size: e.file_size as u64,
        }).collect())
    }

    fn mkdir(&self, _path: &str) -> FsResult<()> {
        Err(FsError::PermissionDenied)
    }

    fn stat(&self, path: &str) -> FsResult<Stat> {
        let info = self.resolve_path(path)?;
        Ok(Stat {
            file_type: if info.is_dir { FileType::Directory } else { FileType::RegularFile },
            size: info.file_size as u64,
            inode: info.cluster as u64,
        })
    }
}

// ---------------------------------------------------------------------------
// In-memory FAT32 image builder (for testing without real disk)
// ---------------------------------------------------------------------------

/// Tạo minimal FAT32 image trong memory để test
pub fn create_test_fat32_image() -> Vec<u8> {
    // 1MB FAT32 image: 2048 sectors × 512 bytes
    let total_sectors: u32 = 2048;
    let mut img = alloc::vec![0u8; total_sectors as usize * 512];

    let sectors_per_cluster: u8 = 8;  // 4KB clusters
    let reserved_sectors: u16 = 32;
    let num_fats: u8 = 2;
    let fat_size: u32 = 4;  // 4 sectors per FAT (enough for 512 entries)
    let root_cluster: u32 = 2;

    // Write BPB to sector 0
    let bpb_offset = 0usize;
    // Jump boot
    img[bpb_offset + 0] = 0xEB;
    img[bpb_offset + 1] = 0x58;
    img[bpb_offset + 2] = 0x90;
    // OEM name
    img[bpb_offset + 3..bpb_offset + 11].copy_from_slice(b"MSDOS5.0");
    // Bytes per sector = 512
    img[bpb_offset + 11] = 0x00;
    img[bpb_offset + 12] = 0x02;
    // Sectors per cluster
    img[bpb_offset + 13] = sectors_per_cluster;
    // Reserved sectors
    img[bpb_offset + 14] = (reserved_sectors & 0xFF) as u8;
    img[bpb_offset + 15] = (reserved_sectors >> 8) as u8;
    // Num FATs
    img[bpb_offset + 16] = num_fats;
    // Root entry count = 0 (FAT32)
    img[bpb_offset + 17] = 0;
    img[bpb_offset + 18] = 0;
    // Total sectors 16 = 0 (FAT32)
    img[bpb_offset + 19] = 0;
    img[bpb_offset + 20] = 0;
    // Media type
    img[bpb_offset + 21] = 0xF8;
    // FAT size 16 = 0 (FAT32)
    img[bpb_offset + 22] = 0;
    img[bpb_offset + 23] = 0;
    // Sectors per track
    img[bpb_offset + 24] = 0x3F;
    img[bpb_offset + 25] = 0x00;
    // Num heads
    img[bpb_offset + 26] = 0xFF;
    img[bpb_offset + 27] = 0x00;
    // Hidden sectors = 0
    img[bpb_offset + 28..bpb_offset + 32].fill(0);
    // Total sectors 32
    img[bpb_offset + 32] = (total_sectors & 0xFF) as u8;
    img[bpb_offset + 33] = ((total_sectors >> 8) & 0xFF) as u8;
    img[bpb_offset + 34] = ((total_sectors >> 16) & 0xFF) as u8;
    img[bpb_offset + 35] = ((total_sectors >> 24) & 0xFF) as u8;
    // FAT size 32
    img[bpb_offset + 36] = (fat_size & 0xFF) as u8;
    img[bpb_offset + 37] = ((fat_size >> 8) & 0xFF) as u8;
    img[bpb_offset + 38] = ((fat_size >> 16) & 0xFF) as u8;
    img[bpb_offset + 39] = ((fat_size >> 24) & 0xFF) as u8;
    // Ext flags
    img[bpb_offset + 40] = 0;
    img[bpb_offset + 41] = 0;
    // FS version
    img[bpb_offset + 42] = 0;
    img[bpb_offset + 43] = 0;
    // Root cluster
    img[bpb_offset + 44] = (root_cluster & 0xFF) as u8;
    img[bpb_offset + 45] = ((root_cluster >> 8) & 0xFF) as u8;
    img[bpb_offset + 46] = ((root_cluster >> 16) & 0xFF) as u8;
    img[bpb_offset + 47] = ((root_cluster >> 24) & 0xFF) as u8;
    // FS info sector
    img[bpb_offset + 48] = 1;
    img[bpb_offset + 49] = 0;
    // Backup boot sector
    img[bpb_offset + 50] = 6;
    img[bpb_offset + 51] = 0;
    // Drive number
    img[bpb_offset + 64] = 0x80;
    // Boot signature
    img[bpb_offset + 66] = 0x29;
    // Volume ID
    img[bpb_offset + 67..bpb_offset + 71].copy_from_slice(&[0x42, 0x42, 0x42, 0x42]);
    // Volume label
    img[bpb_offset + 71..bpb_offset + 82].copy_from_slice(b"MYKERNEL   ");
    // FS type
    img[bpb_offset + 82..bpb_offset + 90].copy_from_slice(b"FAT32   ");
    // Boot signature
    img[510] = 0x55;
    img[511] = 0xAA;

    // Write FAT table (both copies)
    let fat1_offset = reserved_sectors as usize * 512;
    let fat2_offset = fat1_offset + fat_size as usize * 512;

    // FAT[0] = media byte + 0xFFFFFF
    // FAT[1] = EOC
    // FAT[2] = root dir cluster (EOC)
    // FAT[3] = hello.txt cluster (EOC)
    // FAT[4] = subdir cluster (EOC)
    // FAT[5] = subfile cluster (EOC)
    for fat_off in [fat1_offset, fat2_offset] {
        write_fat_entry(&mut img, fat_off, 0, 0x0FFFFFF8);
        write_fat_entry(&mut img, fat_off, 1, 0x0FFFFFFF);
        write_fat_entry(&mut img, fat_off, 2, 0x0FFFFFFF); // root dir
        write_fat_entry(&mut img, fat_off, 3, 0x0FFFFFFF); // hello.txt
        write_fat_entry(&mut img, fat_off, 4, 0x0FFFFFFF); // docs dir
        write_fat_entry(&mut img, fat_off, 5, 0x0FFFFFFF); // readme.txt
    }

    // Data region starts at: reserved + num_fats * fat_size = 32 + 2*4 = 40
    let data_start = (reserved_sectors as u32 + num_fats as u32 * fat_size) as usize * 512;

    // Cluster 2 = root directory (offset = data_start + 0 clusters)
    let cluster_size = sectors_per_cluster as usize * 512;
    let root_dir_off = data_start; // cluster 2

    // Root dir entries:
    // 1. Volume label (MYKERNEL   )
    write_dir_entry(&mut img, root_dir_off, 0,
        b"MYKERNEL   ", ATTR_VOLUME_ID, 0, 0, 0);
    // 2. HELLO.TXT (cluster 3)
    write_dir_entry(&mut img, root_dir_off, 1,
        b"HELLO   TXT", ATTR_ARCHIVE, 3, 26, 0); // "Hello from FAT32!\n" = 18 bytes
    // 3. DOCS (directory, cluster 4)
    write_dir_entry(&mut img, root_dir_off, 2,
        b"DOCS       ", ATTR_DIRECTORY, 4, 0, 0);

    // Cluster 3 = HELLO.TXT content
    let hello_content = b"Hello from FAT32!\nPhase 17: FAT32 Filesystem\n";
    let file_off = data_start + cluster_size; // cluster 3
    img[file_off..file_off + hello_content.len()].copy_from_slice(hello_content);
    // Fix file size in dir entry
    let hello_size = hello_content.len() as u32;
    let size_off = root_dir_off + 32 + 28; // entry 1, offset 28 = file_size
    img[size_off]   = (hello_size & 0xFF) as u8;
    img[size_off+1] = ((hello_size >> 8) & 0xFF) as u8;
    img[size_off+2] = ((hello_size >> 16) & 0xFF) as u8;
    img[size_off+3] = ((hello_size >> 24) & 0xFF) as u8;

    // Cluster 4 = DOCS directory
    let docs_off = data_start + cluster_size * 2; // cluster 4
    // . entry
    write_dir_entry(&mut img, docs_off, 0, b".          ", ATTR_DIRECTORY, 4, 0, 0);
    // .. entry
    write_dir_entry(&mut img, docs_off, 1, b"..         ", ATTR_DIRECTORY, 2, 0, 0);
    // README.TXT (cluster 5)
    write_dir_entry(&mut img, docs_off, 2,
        b"README  TXT", ATTR_ARCHIVE, 5, 37, 0);

    // Cluster 5 = README.TXT
    let readme = b"MyKernel FAT32 test filesystem\nPhase 17\n";
    let readme_off = data_start + cluster_size * 3;
    img[readme_off..readme_off + readme.len()].copy_from_slice(readme);
    let readme_size = readme.len() as u32;
    let rsize_off = docs_off + 2*32 + 28;
    img[rsize_off]   = (readme_size & 0xFF) as u8;
    img[rsize_off+1] = ((readme_size >> 8) & 0xFF) as u8;
    img[rsize_off+2] = ((readme_size >> 16) & 0xFF) as u8;
    img[rsize_off+3] = ((readme_size >> 24) & 0xFF) as u8;

    img
}

fn write_fat_entry(img: &mut [u8], fat_off: usize, cluster: usize, value: u32) {
    let off = fat_off + cluster * 4;
    img[off]   = (value & 0xFF) as u8;
    img[off+1] = ((value >> 8) & 0xFF) as u8;
    img[off+2] = ((value >> 16) & 0xFF) as u8;
    img[off+3] = ((value >> 24) & 0xFF) as u8;
}

fn write_dir_entry(img: &mut [u8], dir_off: usize, idx: usize,
    name11: &[u8; 11], attr: u8, cluster: u32, size: u32, _reserved: u32)
{
    let off = dir_off + idx * 32;
    img[off..off+11].copy_from_slice(name11);
    img[off+11] = attr;
    // fst_clus_hi at offset 20
    img[off+20] = ((cluster >> 16) & 0xFF) as u8;
    img[off+21] = ((cluster >> 24) & 0xFF) as u8;
    // fst_clus_lo at offset 26
    img[off+26] = (cluster & 0xFF) as u8;
    img[off+27] = ((cluster >> 8) & 0xFF) as u8;
    // file_size at offset 28
    img[off+28] = (size & 0xFF) as u8;
    img[off+29] = ((size >> 8) & 0xFF) as u8;
    img[off+30] = ((size >> 16) & 0xFF) as u8;
    img[off+31] = ((size >> 24) & 0xFF) as u8;
    // Date/time fields (dummy values)
    img[off+14] = 0x00; // crt_time_tenth
    img[off+16] = 0x20; // wrt_time
    img[off+17] = 0x4A;
    img[off+18] = 0x4A; // wrt_date
    img[off+19] = 0x4A;
}

// ---------------------------------------------------------------------------
// RAM-backed block device (for testing)
// ---------------------------------------------------------------------------

pub struct RamBlockDevice {
    data: Mutex<Vec<u8>>,
}

impl RamBlockDevice {
    pub fn new(data: Vec<u8>) -> Arc<Self> {
        Arc::new(RamBlockDevice { data: Mutex::new(data) })
    }
}

impl BlockDevice for RamBlockDevice {
    fn read_sector(&self, sector: u64, buf: &mut [u8; 512]) -> FsResult<()> {
        let data = self.data.lock();
        let offset = sector as usize * 512;
        if offset + 512 > data.len() {
            return Err(FsError::InvalidArgument);
        }
        buf.copy_from_slice(&data[offset..offset + 512]);
        Ok(())
    }

    fn write_sector(&self, sector: u64, buf: &[u8; 512]) -> FsResult<()> {
        let mut data = self.data.lock();
        let offset = sector as usize * 512;
        if offset + 512 > data.len() {
            return Err(FsError::InvalidArgument);
        }
        data[offset..offset + 512].copy_from_slice(buf);
        Ok(())
    }
}
