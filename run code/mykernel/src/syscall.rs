/// Syscall Table — Linux x86_64 ABI Compatible
///
/// musl libc cần các syscalls sau để hoạt động:
///
/// **Process:**
///   exit(1), exit_group(231), getpid(39), getppid(110)
///   fork(57) — optional
///
/// **Memory:**
///   brk(12), mmap(9), munmap(11), mprotect(10)
///
/// **File I/O:**
///   read(0), write(1), open(2), close(3)
///   stat(4), fstat(5), lstat(6)
///   lseek(8)
///   ioctl(16)
///   access(21)
///   dup(32), dup2(33)
///   fcntl(72)
///   readlink(89)
///
/// **Directory:**
///   getdents64(217)
///   getcwd(79)
///   chdir(80)
///
/// **Time:**
///   gettimeofday(96)
///   clock_gettime(228)
///   nanosleep(35)
///
/// **Other:**
///   uname(63)
///   set_tid_address(218)
///   set_robust_list(273)
///   rt_sigaction(13), rt_sigprocmask(14)
///   arch_prctl(158)
///   futex(202)
///   prlimit64(302)
///   getrandom(318)

use alloc::string::String;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Syscall numbers (Linux x86_64 ABI)
// ---------------------------------------------------------------------------

pub const SYS_READ:          u64 = 0;
pub const SYS_WRITE:         u64 = 1;
pub const SYS_OPEN:          u64 = 2;
pub const SYS_CLOSE:         u64 = 3;
pub const SYS_STAT:          u64 = 4;
pub const SYS_FSTAT:         u64 = 5;
pub const SYS_LSTAT:         u64 = 6;
pub const SYS_LSEEK:         u64 = 8;
pub const SYS_MMAP:          u64 = 9;
pub const SYS_MPROTECT:      u64 = 10;
pub const SYS_MUNMAP:        u64 = 11;
pub const SYS_BRK:           u64 = 12;
pub const SYS_RT_SIGACTION:  u64 = 13;
pub const SYS_RT_SIGPROCMASK:u64 = 14;
pub const SYS_IOCTL:         u64 = 16;
pub const SYS_ACCESS:        u64 = 21;
pub const SYS_NANOSLEEP:     u64 = 35;
pub const SYS_GETPID:        u64 = 39;
pub const SYS_FORK:          u64 = 57;
pub const SYS_EXECVE:        u64 = 59;
pub const SYS_EXIT:          u64 = 60;
pub const SYS_WAIT4:         u64 = 61;
pub const SYS_UNAME:         u64 = 63;
pub const SYS_FCNTL:         u64 = 72;
pub const SYS_GETCWD:        u64 = 79;
pub const SYS_CHDIR:         u64 = 80;
pub const SYS_READLINK:      u64 = 89;
pub const SYS_GETPPID:       u64 = 110;
pub const SYS_GETTIMEOFDAY:  u64 = 96;
pub const SYS_GETUID:        u64 = 102;
pub const SYS_GETGID:        u64 = 104;
pub const SYS_GETEUID:       u64 = 107;
pub const SYS_GETEGID:       u64 = 108;
pub const SYS_ARCH_PRCTL:    u64 = 158;
pub const SYS_FUTEX:         u64 = 202;
pub const SYS_GETDENTS64:    u64 = 217;
pub const SYS_SET_TID_ADDRESS:u64 = 218;
pub const SYS_CLOCK_GETTIME: u64 = 228;
pub const SYS_EXIT_GROUP:    u64 = 231;
pub const SYS_SET_ROBUST_LIST:u64 = 273;
pub const SYS_PRLIMIT64:     u64 = 302;
pub const SYS_GETRANDOM:     u64 = 318;

// ---------------------------------------------------------------------------
// Error codes (Linux errno)
// ---------------------------------------------------------------------------

pub const EPERM:   i64 = -1;
pub const ENOENT:  i64 = -2;
pub const EINTR:   i64 = -4;
pub const EBADF:   i64 = -9;
pub const ENOMEM:  i64 = -12;
pub const EACCES:  i64 = -13;
pub const EFAULT:  i64 = -14;
pub const ENOTDIR: i64 = -20;
pub const EINVAL:  i64 = -22;
pub const ENOSYS:  i64 = -38;
pub const ENOTSUP: i64 = -95;

// ---------------------------------------------------------------------------
// open() flags
// ---------------------------------------------------------------------------
pub const O_RDONLY:  u64 = 0;
pub const O_WRONLY:  u64 = 1;
pub const O_RDWR:    u64 = 2;
pub const O_CREAT:   u64 = 0o100;
pub const O_TRUNC:   u64 = 0o1000;
pub const O_APPEND:  u64 = 0o2000;
pub const O_NONBLOCK:u64 = 0o4000;
pub const O_CLOEXEC: u64 = 0o2000000;

// ---------------------------------------------------------------------------
// mmap() flags
// ---------------------------------------------------------------------------
pub const PROT_READ:  u64 = 1;
pub const PROT_WRITE: u64 = 2;
pub const PROT_EXEC:  u64 = 4;
pub const MAP_SHARED:  u64 = 1;
pub const MAP_PRIVATE: u64 = 2;
pub const MAP_ANON:    u64 = 0x20;
pub const MAP_FIXED:   u64 = 0x10;
pub const MAP_FAILED:  u64 = u64::MAX; // (void*)-1

// ---------------------------------------------------------------------------
// arch_prctl() codes
// ---------------------------------------------------------------------------
pub const ARCH_SET_FS: u64 = 0x1002;
pub const ARCH_GET_FS: u64 = 0x1003;
pub const ARCH_SET_GS: u64 = 0x1001;
pub const ARCH_GET_GS: u64 = 0x1004;

// ---------------------------------------------------------------------------
// Linux stat structure (x86_64)
// ---------------------------------------------------------------------------
#[repr(C)]
pub struct LinuxStat {
    pub st_dev:     u64,
    pub st_ino:     u64,
    pub st_nlink:   u64,
    pub st_mode:    u32,
    pub st_uid:     u32,
    pub st_gid:     u32,
    pub _pad0:      u32,
    pub st_rdev:    u64,
    pub st_size:    i64,
    pub st_blksize: i64,
    pub st_blocks:  i64,
    pub st_atime:   u64,
    pub st_atime_ns:u64,
    pub st_mtime:   u64,
    pub st_mtime_ns:u64,
    pub st_ctime:   u64,
    pub st_ctime_ns:u64,
    pub _unused:    [i64; 3],
}

// ---------------------------------------------------------------------------
// uname structure
// ---------------------------------------------------------------------------
#[repr(C)]
pub struct Utsname {
    pub sysname:    [u8; 65],
    pub nodename:   [u8; 65],
    pub release:    [u8; 65],
    pub version:    [u8; 65],
    pub machine:    [u8; 65],
    pub domainname: [u8; 65],
}

// ---------------------------------------------------------------------------
// Process memory state (for brk/mmap)
// ---------------------------------------------------------------------------

static HEAP_END: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static MMAP_BASE: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0x7000_0000_0000);

pub fn init_process_memory(initial_brk: u64) {
    HEAP_END.store(initial_brk, core::sync::atomic::Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Main syscall dispatcher
// ---------------------------------------------------------------------------

/// Dispatch syscall — called from usermode.rs syscall_handler
pub fn dispatch(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> i64 {
    match nr {
        SYS_READ        => sys_read(a1, a2, a3),
        SYS_WRITE       => sys_write(a1, a2, a3),
        SYS_OPEN        => sys_open(a1, a2, a3),
        SYS_CLOSE       => sys_close(a1),
        SYS_STAT        => sys_stat(a1, a2),
        SYS_FSTAT       => sys_fstat(a1, a2),
        SYS_LSTAT       => sys_stat(a1, a2), // same as stat for now
        SYS_LSEEK       => sys_lseek(a1, a2 as i64, a3),
        SYS_MMAP        => sys_mmap(a1, a2, a3, a4, a5 as i64, a6),
        SYS_MPROTECT    => 0, // Ignore mprotect for now
        SYS_MUNMAP      => 0, // Ignore munmap for now
        SYS_BRK         => sys_brk(a1),
        SYS_RT_SIGACTION  => 0, // Return success (no signals yet)
        SYS_RT_SIGPROCMASK => 0,
        SYS_IOCTL       => sys_ioctl(a1, a2, a3),
        SYS_ACCESS      => sys_access(a1, a2),
        SYS_NANOSLEEP   => 0, // Return immediately
        SYS_GETPID      => 1,
        SYS_GETPPID     => 0,
        SYS_GETUID      => 0,
        SYS_GETGID      => 0,
        SYS_GETEUID     => 0,
        SYS_GETEGID     => 0,
        SYS_FORK        => ENOSYS,
        SYS_EXIT        => sys_exit(a1),
        SYS_EXIT_GROUP  => sys_exit(a1),
        SYS_UNAME       => sys_uname(a1),
        SYS_FCNTL       => sys_fcntl(a1, a2, a3),
        SYS_GETCWD      => sys_getcwd(a1, a2),
        SYS_CHDIR       => 0,
        SYS_READLINK    => ENOENT,
        SYS_GETTIMEOFDAY => sys_gettimeofday(a1, a2),
        SYS_ARCH_PRCTL  => sys_arch_prctl(a1, a2),
        SYS_FUTEX       => sys_futex(a1, a2, a3),
        SYS_GETDENTS64  => sys_getdents64(a1, a2, a3),
        SYS_SET_TID_ADDRESS => 1,
        SYS_CLOCK_GETTIME => sys_clock_gettime(a1, a2),
        SYS_SET_ROBUST_LIST => 0,
        SYS_PRLIMIT64   => sys_prlimit64(a1, a2, a3, a4),
        SYS_GETRANDOM   => sys_getrandom(a1, a2, a3),
        _ => {
            crate::serial_println!("[syscall] ENOSYS: nr={}", nr);
            ENOSYS
        }
    }
}

// ---------------------------------------------------------------------------
// File descriptor table (global — will be per-process in future)
// ---------------------------------------------------------------------------

use spin::Mutex;
use alloc::sync::Arc;
use crate::fs::vfs::{File, OpenFlags};

const MAX_FDS: usize = 256;

struct FdTable {
    files: [Option<Arc<Mutex<dyn File>>>; MAX_FDS],
}

impl FdTable {
    const fn new() -> Self {
        // Can't use array init with non-Copy types easily in const context
        // We'll use unsafe or a workaround
        FdTable { files: [const { None }; MAX_FDS] }
    }

    fn get(&self, fd: usize) -> Option<Arc<Mutex<dyn File>>> {
        if fd >= MAX_FDS { return None; }
        self.files[fd].as_ref().map(Arc::clone)
    }

    fn insert(&mut self, file: Arc<Mutex<dyn File>>) -> Option<usize> {
        for i in 3..MAX_FDS {
            if self.files[i].is_none() {
                self.files[i] = Some(file);
                return Some(i);
            }
        }
        None
    }

    fn close(&mut self, fd: usize) -> bool {
        if fd >= MAX_FDS { return false; }
        self.files[fd].take().is_some()
    }
}

static FD_TABLE: Mutex<FdTable> = Mutex::new(FdTable::new());

// ---------------------------------------------------------------------------
// Syscall implementations
// ---------------------------------------------------------------------------

fn sys_read(fd: u64, buf_ptr: u64, len: u64) -> i64 {
    let fd = fd as usize;
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };

    let file = match FD_TABLE.lock().get(fd) {
        Some(f) => f,
        None => return EBADF,
    };

    let x = match file.lock().read(buf) {
        Ok(n) => n as i64,
        Err(crate::fs::FsError::EndOfFile) => 0,
        Err(_) => EBADF,
    }; x
}

fn sys_write(fd: u64, buf_ptr: u64, len: u64) -> i64 {
    // fd 1 (stdout) and fd 2 (stderr) → VGA + serial
    if fd == 1 || fd == 2 {
        let bytes = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
        for &b in bytes {
            if b.is_ascii() && (b >= 0x20 || b == b'\n' || b == b'\r' || b == b'\t') {
                crate::print!("{}", b as char);
            }
        }
        crate::serial_println!("[sys_write] fd={} len={}", fd, len);
        return len as i64;
    }

    let fd_n = fd as usize;
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };

    let file = match FD_TABLE.lock().get(fd_n) {
        Some(f) => f,
        None => return EBADF,
    };

    let x = match file.lock().write(buf) {
        Ok(n) => n as i64,
        Err(_) => EBADF,
    }; x
}

fn sys_open(path_ptr: u64, flags: u64, _mode: u64) -> i64 {
    let path = unsafe {
        let ptr = path_ptr as *const u8;
        let mut len = 0;
        while *ptr.add(len) != 0 { len += 1; }
        core::str::from_utf8(core::slice::from_raw_parts(ptr, len)).unwrap_or("")
    };

    crate::serial_println!("[sys_open] path={} flags={:#o}", path, flags);

    let open_flags = if flags & O_WRONLY != 0 || flags & O_RDWR != 0 {
        if flags & O_CREAT != 0 { OpenFlags::WRITE }
        else { OpenFlags::WRITE }
    } else {
        OpenFlags::READ
    };

    let file = if flags & O_CREAT != 0 {
        crate::fs::create(path).or_else(|_| crate::fs::open(path, open_flags))
    } else {
        crate::fs::open(path, open_flags)
    };

    match file {
        Ok(f) => {
            match FD_TABLE.lock().insert(f) {
                Some(fd) => fd as i64,
                None => ENOMEM,
            }
        }
        Err(crate::fs::FsError::NotFound) => ENOENT,
        Err(crate::fs::FsError::PermissionDenied) => EACCES,
        Err(_) => EINVAL,
    }
}

fn sys_close(fd: u64) -> i64 {
    if fd < 3 { return 0; } // Don't close stdin/stdout/stderr
    if FD_TABLE.lock().close(fd as usize) { 0 } else { EBADF }
}

fn sys_stat(path_ptr: u64, stat_ptr: u64) -> i64 {
    let path = read_cstr(path_ptr);
    crate::serial_println!("[sys_stat] path={}", path);

    match crate::fs::stat(&path) {
        Ok(s) => {
            let stat = unsafe { &mut *(stat_ptr as *mut LinuxStat) };
            *stat = LinuxStat {
                st_dev: 1,
                st_ino: s.inode,
                st_nlink: 1,
                st_mode: match s.file_type {
                    crate::fs::FileType::Directory => 0o040755,
                    _ => 0o100644,
                },
                st_uid: 0,
                st_gid: 0,
                _pad0: 0,
                st_rdev: 0,
                st_size: s.size as i64,
                st_blksize: 512,
                st_blocks: ((s.size + 511) / 512) as i64,
                st_atime: 0, st_atime_ns: 0,
                st_mtime: 0, st_mtime_ns: 0,
                st_ctime: 0, st_ctime_ns: 0,
                _unused: [0; 3],
            };
            0
        }
        Err(crate::fs::FsError::NotFound) => ENOENT,
        Err(_) => EINVAL,
    }
}

fn sys_fstat(fd: u64, stat_ptr: u64) -> i64 {
    let file = match FD_TABLE.lock().get(fd as usize) {
        Some(f) => f,
        None => return EBADF,
    };

    let x = match file.lock().stat() {
        Ok(s) => {
            let stat = unsafe { &mut *(stat_ptr as *mut LinuxStat) };
            *stat = LinuxStat {
                st_dev: 1, st_ino: s.inode, st_nlink: 1,
                st_mode: 0o100644, st_uid: 0, st_gid: 0, _pad0: 0,
                st_rdev: 0, st_size: s.size as i64, st_blksize: 512,
                st_blocks: ((s.size + 511) / 512) as i64,
                st_atime: 0, st_atime_ns: 0,
                st_mtime: 0, st_mtime_ns: 0,
                st_ctime: 0, st_ctime_ns: 0,
                _unused: [0; 3],
            };
            0
        }
        Err(_) => EBADF,
    }; x
}

fn sys_lseek(fd: u64, offset: i64, whence: u64) -> i64 {
    let file = match FD_TABLE.lock().get(fd as usize) {
        Some(f) => f,
        None => return EBADF,
    };

    let w = match whence {
        0 => crate::fs::vfs::SeekWhence::Set,
        1 => crate::fs::vfs::SeekWhence::Cur,
        2 => crate::fs::vfs::SeekWhence::End,
        _ => return EINVAL,
    };

    let x = match file.lock().seek(offset, w) {
        Ok(pos) => pos as i64,
        Err(_) => EINVAL,
    }; x
}

fn sys_mmap(addr: u64, len: u64, _prot: u64, flags: u64, _fd: i64, _offset: u64) -> i64 {
    // Simple anonymous mmap — allocate from a bump allocator region
    if flags & MAP_ANON == 0 {
        crate::serial_println!("[sys_mmap] file-backed mmap not supported");
        return MAP_FAILED as i64;
    }

    // Allocate from mmap region (grows downward from 0x7000_0000_0000)
    let base = MMAP_BASE.fetch_sub(
        (len + 4095) & !4095,
        core::sync::atomic::Ordering::Relaxed
    );

    crate::serial_println!("[sys_mmap] anon mmap at {:#x} len={}", base - len, len);
    (base - ((len + 4095) & !4095)) as i64
}

fn sys_brk(addr: u64) -> i64 {
    let current = HEAP_END.load(core::sync::atomic::Ordering::Relaxed);
    if addr == 0 || addr < current {
        return current as i64;
    }
    HEAP_END.store(addr, core::sync::atomic::Ordering::Relaxed);
    addr as i64
}

fn sys_ioctl(fd: u64, request: u64, _arg: u64) -> i64 {
    // TIOCGWINSZ = 0x5413 (get terminal size)
    if request == 0x5413 {
        return 0; // Return success with zero-filled struct
    }
    crate::serial_println!("[sys_ioctl] fd={} req={:#x}", fd, request);
    EINVAL
}

fn sys_access(path_ptr: u64, _mode: u64) -> i64 {
    let path = read_cstr(path_ptr);
    match crate::fs::stat(&path) {
        Ok(_) => 0,
        Err(_) => ENOENT,
    }
}

fn sys_exit(code: u64) -> i64 {
    crate::println!("[user] exit({})", code);
    crate::serial_println!("[syscall] exit({})", code);
    loop { x86_64::instructions::hlt(); }
}

fn sys_uname(buf_ptr: u64) -> i64 {
    let uname = unsafe { &mut *(buf_ptr as *mut Utsname) };
    fill_cstr(&mut uname.sysname,    b"Linux");
    fill_cstr(&mut uname.nodename,   b"mykernel");
    fill_cstr(&mut uname.release,    b"6.1.0-mykernel");
    fill_cstr(&mut uname.version,    b"#1 Rust Bare-Metal");
    fill_cstr(&mut uname.machine,    b"x86_64");
    fill_cstr(&mut uname.domainname, b"localdomain");
    0
}

fn sys_fcntl(fd: u64, cmd: u64, _arg: u64) -> i64 {
    match cmd {
        1 => 0,  // F_GETFD → return 0 (no close-on-exec)
        2 => 0,  // F_SETFD → success
        3 => if fd == 0 { O_RDONLY as i64 } else { (O_WRONLY | O_APPEND) as i64 }, // F_GETFL
        4 => 0,  // F_SETFL → success
        _ => EINVAL,
    }
}

fn sys_getcwd(buf_ptr: u64, size: u64) -> i64 {
    let cwd = b"/\0";
    if size < cwd.len() as u64 { return EINVAL; }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, size as usize) };
    buf[0] = b'/';
    buf[1] = 0;
    buf_ptr as i64
}

fn sys_gettimeofday(tv_ptr: u64, _tz_ptr: u64) -> i64 {
    if tv_ptr != 0 {
        let tv = unsafe { &mut *(tv_ptr as *mut [u64; 2]) };
        tv[0] = 0; // seconds since epoch
        tv[1] = 0; // microseconds
    }
    0
}

fn sys_arch_prctl(code: u64, addr: u64) -> i64 {
    match code {
        ARCH_SET_FS => {
            // Set FS base register (used by musl for TLS)
            unsafe {
                x86_64::registers::model_specific::Msr::new(0xC0000100).write(addr);
            }
            crate::serial_println!("[arch_prctl] ARCH_SET_FS addr={:#x}", addr);
            0
        }
        ARCH_GET_FS => {
            let addr_ptr = unsafe { &mut *(addr as *mut u64) };
            *addr_ptr = unsafe { x86_64::registers::model_specific::Msr::new(0xC0000100).read() };
            0
        }
        ARCH_SET_GS => {
            unsafe {
                x86_64::registers::model_specific::Msr::new(0xC0000101).write(addr);
            }
            0
        }
        _ => EINVAL,
    }
}

fn sys_futex(uaddr: u64, op: u64, _val: u64) -> i64 {
    // FUTEX_WAIT=0, FUTEX_WAKE=1
    // Simple: WAIT returns immediately, WAKE returns 0
    crate::serial_println!("[sys_futex] addr={:#x} op={}", uaddr, op);
    0
}

fn sys_getdents64(fd: u64, buf_ptr: u64, count: u64) -> i64 {
    // Simplified getdents64 — not fully implemented
    crate::serial_println!("[sys_getdents64] fd={}", fd);
    0 // EOF
}

fn sys_clock_gettime(clockid: u64, tp_ptr: u64) -> i64 {
    if tp_ptr != 0 {
        let tp = unsafe { &mut *(tp_ptr as *mut [u64; 2]) };
        tp[0] = 0; // seconds
        tp[1] = 0; // nanoseconds
    }
    0
}

fn sys_prlimit64(pid: u64, resource: u64, new_limit: u64, old_limit: u64) -> i64 {
    // Return "unlimited" for all resources
    if old_limit != 0 {
        let rlim = unsafe { &mut *(old_limit as *mut [u64; 2]) };
        rlim[0] = u64::MAX; // rlim_cur = unlimited
        rlim[1] = u64::MAX; // rlim_max = unlimited
    }
    0
}

fn sys_getrandom(buf_ptr: u64, len: u64, _flags: u64) -> i64 {
    // Simple pseudo-random using RDTSC
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
    for (i, b) in buf.iter_mut().enumerate() {
        // Simple LFSR-based pseudorandom
        *b = (i.wrapping_mul(0x1F + 1) ^ 0xA5) as u8;
    }
    len as i64
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_cstr(ptr: u64) -> String {
    if ptr == 0 { return String::new(); }
    unsafe {
        let p = ptr as *const u8;
        let mut len = 0;
        while *p.add(len) != 0 && len < 4096 { len += 1; }
        String::from_utf8_lossy(core::slice::from_raw_parts(p, len)).into_owned()
    }
}

fn fill_cstr(dst: &mut [u8], src: &[u8]) {
    let len = src.len().min(dst.len() - 1);
    dst[..len].copy_from_slice(&src[..len]);
    dst[len] = 0;
}
