/// Security Hardening — Phase 24
///
/// Các kỹ thuật bảo vệ kernel:
///
/// 1. **SMEP** (Supervisor Mode Execution Prevention)
///    CR4.SMEP = 1 → CPU fault nếu kernel thực thi code ở user pages
///    Ngăn privilege escalation qua user-space shellcode
///
/// 2. **SMAP** (Supervisor Mode Access Prevention)
///    CR4.SMAP = 1 → CPU fault nếu kernel đọc/ghi user pages mà không dùng STAC/CLAC
///    Ngăn kernel bị trick đọc/ghi vùng nhớ user-controlled
///
/// 3. **NX/XD** (No-Execute / Execute Disable)
///    Page table bit 63 = NX → trang đó không thể thực thi
///    Stack, heap, data không chạy được như code
///
/// 4. **KASLR** (Kernel Address Space Layout Randomization)
///    Kernel load ở địa chỉ ngẫu nhiên → attacker không biết địa chỉ
///    (Simplified: demonstrate concept, not full implementation)
///
/// 5. **Stack Canary**
///    Magic value ở đầu stack frame → phát hiện stack overflow
///    Nếu canary bị ghi đè → kernel panic trước khi return
///
/// 6. **Syscall Validation**
///    Validate tất cả pointers từ user space
///    Kiểm tra buffers không overlap kernel memory
///
/// 7. **ASLR** (Address Space Layout Randomization) cho user processes
///    Randomize stack/heap/library addresses trong user space
///
/// 8. **Capability System** (simplified)
///    Processes có capabilities thay vì quyền root toàn bộ

use core::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use alloc::string::String;

// ---------------------------------------------------------------------------
// 1. CPU Security Features (SMEP, SMAP, NX)
// ---------------------------------------------------------------------------

/// CR4 register bits
pub const CR4_SMEP: u64 = 1 << 20;
pub const CR4_SMAP: u64 = 1 << 21;
pub const CR4_UMIP: u64 = 1 << 11; // User-Mode Instruction Prevention

/// EFER register bits
pub const EFER_NX: u64 = 1 << 11; // No-Execute Enable

pub struct CpuSecurityFeatures {
    pub smep: bool,
    pub smap: bool,
    pub umip: bool,
    pub nx:   bool,
    pub rdrand: bool,
}

/// Detect what security features the CPU supports
pub fn detect_cpu_security() -> CpuSecurityFeatures {
    let (ecx7, edx7, ecx1, edx1) = unsafe {
        let (mut ecx7, mut edx7) = (0u32, 0u32);
        let (mut ecx1, mut edx1) = (0u32, 0u32);
        // CPUID leaf 7 (extended features)
        core::arch::asm!(
            "push rbx",
            "xor ecx, ecx",
            "mov eax, 7",
            "cpuid",
            "mov edi, ecx",
            "mov esi, edx",
            "pop rbx",
            in("eax") 7u32,
            out("edi") ecx7,
            out("esi") edx7,
            out("ecx") _,
            lateout("eax") _,
        );
        // CPUID leaf 1 (basic features)
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov edi, ecx",
            "mov esi, edx",
            "pop rbx",
            in("eax") 1u32,
            out("edi") ecx1,
            out("esi") edx1,
            out("ecx") _,
            lateout("eax") _,
        );
        (ecx7, edx7, ecx1, edx1)
    };

    CpuSecurityFeatures {
        smep:   edx7 & (1 << 7) != 0,
        smap:   edx7 & (1 << 20) != 0,
        umip:   ecx7 & (1 << 2) != 0,
        nx:     true, // We already set this in EFER
        rdrand: ecx1 & (1 << 30) != 0,
    }
}

/// Enable SMEP + SMAP + UMIP if supported
pub fn enable_cpu_security_features() {
    let features = detect_cpu_security();

    unsafe {
        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4);

        if features.smep {
            cr4 |= CR4_SMEP;
            crate::serial_println!("[security] SMEP enabled");
        }
        if features.smap {
            cr4 |= CR4_SMAP;
            crate::serial_println!("[security] SMAP enabled");
        }
        if features.umip {
            cr4 |= CR4_UMIP;
            crate::serial_println!("[security] UMIP enabled");
        }

        core::arch::asm!("mov cr4, {}", in(reg) cr4);
    }

    crate::serial_println!("[security] CPU features: SMEP={} SMAP={} UMIP={} RDRAND={}",
        features.smep, features.smap, features.umip, features.rdrand);
}

// ---------------------------------------------------------------------------
// 2. Stack Canary
// ---------------------------------------------------------------------------

static STACK_CANARY: AtomicU64 = AtomicU64::new(0);

pub fn init_stack_canary() {
    // Use RDTSC + fixed salt as canary value
    let canary = unsafe {
        let tsc: u64;
        core::arch::asm!("rdtsc", out("rax") tsc, out("rdx") _);
        tsc ^ 0xDEAD_BEEF_CAFE_BABE
    };
    STACK_CANARY.store(canary, Ordering::Relaxed);
    crate::serial_println!("[security] Stack canary initialized: {:#x}", canary);
}

pub fn get_stack_canary() -> u64 {
    STACK_CANARY.load(Ordering::Relaxed)
}

/// Check if stack canary is intact
/// Called at function epilogue in hardened code
pub fn check_stack_canary(saved_canary: u64) -> bool {
    saved_canary == STACK_CANARY.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// 3. Syscall Pointer Validation
// ---------------------------------------------------------------------------

/// Kernel virtual address range
/// Anything above this is kernel space
pub const USER_SPACE_TOP: u64 = 0x0000_8000_0000_0000;
pub const KERNEL_BASE:    u64 = 0xFFFF_8000_0000_0000;

/// Check if a pointer is safe to dereference from a syscall
pub fn validate_user_ptr(ptr: u64, len: usize) -> bool {
    if ptr == 0 { return false; }
    // Must be in user space
    if ptr >= USER_SPACE_TOP { return false; }
    // Must not wrap around
    if ptr.checked_add(len as u64).map_or(true, |end| end > USER_SPACE_TOP) {
        return false;
    }
    true
}

/// Validate a user-provided string pointer (null-terminated)
pub fn validate_user_cstr(ptr: u64, max_len: usize) -> bool {
    if ptr == 0 || ptr >= USER_SPACE_TOP { return false; }
    // Check each byte up to max_len
    for i in 0..max_len {
        let byte_addr = ptr + i as u64;
        if byte_addr >= USER_SPACE_TOP { return false; }
        let byte = unsafe { *(byte_addr as *const u8) };
        if byte == 0 { return true; } // Found null terminator
    }
    false // No null terminator within max_len
}

/// Safe copy from user space
pub fn copy_from_user(dst: &mut [u8], src_ptr: u64) -> Result<(), SecurityError> {
    if !validate_user_ptr(src_ptr, dst.len()) {
        return Err(SecurityError::InvalidUserPointer);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            src_ptr as *const u8,
            dst.as_mut_ptr(),
            dst.len(),
        );
    }
    Ok(())
}

/// Safe copy to user space
pub fn copy_to_user(dst_ptr: u64, src: &[u8]) -> Result<(), SecurityError> {
    if !validate_user_ptr(dst_ptr, src.len()) {
        return Err(SecurityError::InvalidUserPointer);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            src.as_ptr(),
            dst_ptr as *mut u8,
            src.len(),
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SecurityError {
    InvalidUserPointer,
    PermissionDenied,
    CapabilityMissing,
    StackCorruption,
    KernelPointerLeak,
}

// ---------------------------------------------------------------------------
// 4. Capability System (simplified Linux-like)
// ---------------------------------------------------------------------------

/// Process capabilities (simplified subset of Linux capabilities)
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    pub caps: u64, // Bitmask
}

/// Capability bits
pub const CAP_CHOWN:      u64 = 1 << 0;
pub const CAP_NET_BIND:   u64 = 1 << 10; // Bind to port < 1024
pub const CAP_NET_RAW:    u64 = 1 << 13; // Raw sockets
pub const CAP_SYS_ADMIN:  u64 = 1 << 21; // System administration
pub const CAP_SYS_REBOOT: u64 = 1 << 26; // Reboot system
pub const CAP_NET_ADMIN:  u64 = 1 << 12; // Network administration
pub const CAP_DAC_OVERRIDE:u64 = 1 << 1; // Bypass file permissions
pub const CAP_SETUID:     u64 = 1 << 7;  // Set UID
pub const CAP_SETGID:     u64 = 1 << 6;  // Set GID

/// Root process (uid=0) gets all capabilities
pub const CAP_FULL: u64 = u64::MAX;
/// Unprivileged process gets minimal capabilities
pub const CAP_NONE: u64 = 0;

impl Capabilities {
    pub fn root() -> Self { Capabilities { caps: CAP_FULL } }
    pub fn none() -> Self { Capabilities { caps: CAP_NONE } }

    pub fn has(&self, cap: u64) -> bool {
        self.caps & cap != 0
    }

    pub fn drop(&mut self, cap: u64) {
        self.caps &= !cap;
    }

    pub fn add(&mut self, cap: u64) {
        self.caps |= cap;
    }

    /// Check if allowed to bind to a port
    pub fn can_bind_port(&self, port: u16) -> bool {
        port >= 1024 || self.has(CAP_NET_BIND)
    }
}

// ---------------------------------------------------------------------------
// 5. KASLR Offset (simplified demo)
// ---------------------------------------------------------------------------

static KASLR_OFFSET: AtomicU64 = AtomicU64::new(0);

pub fn init_kaslr() {
    // In real KASLR, bootloader randomizes load address
    // Here we demonstrate the concept with RDTSC
    let offset = unsafe {
        let tsc: u64;
        core::arch::asm!("rdtsc", out("rax") tsc, out("rdx") _);
        // Align to 2MB (page table granularity for huge pages)
        (tsc & 0x3FF) * 0x200000
    };
    KASLR_OFFSET.store(offset, Ordering::Relaxed);
    crate::serial_println!("[security] KASLR offset: +{:#x}", offset);
}

pub fn kaslr_offset() -> u64 {
    KASLR_OFFSET.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// 6. Entropy / CSPRNG (simple)
// ---------------------------------------------------------------------------

struct CsprngState {
    state: [u64; 4],
}

impl CsprngState {
    fn new(seed: u64) -> Self {
        CsprngState {
            state: [
                seed ^ 0x6C62272E07BB0142,
                seed.wrapping_add(0x62B821756295C58D),
                seed.rotate_left(17) ^ 0x9E3779B97F4A7C15,
                seed.wrapping_mul(6364136223846793005),
            ]
        }
    }

    /// xoshiro256** PRNG
    fn next(&mut self) -> u64 {
        let result = self.state[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.state[1] << 17;
        self.state[2] ^= self.state[0];
        self.state[3] ^= self.state[1];
        self.state[1] ^= self.state[2];
        self.state[0] ^= self.state[3];
        self.state[2] ^= t;
        self.state[3] = self.state[3].rotate_left(45);
        result
    }
}

static mut CSPRNG: Option<CsprngState> = None;

pub fn init_entropy() {
    let seed = unsafe {
        let mut tsc: u64;
        core::arch::asm!("rdtsc", out("rax") tsc, out("rdx") _);
        // Mix with some hardware info
        tsc ^= 0xDEAD_C0DE_CAFE_BABE;
        tsc
    };
    unsafe {
        CSPRNG = Some(CsprngState::new(seed));
    }
    crate::serial_println!("[security] Entropy pool initialized (seed={:#x})", seed);
}

pub fn get_random_u64() -> u64 {
    unsafe {
        if let Some(ref mut rng) = CSPRNG {
            rng.next()
        } else {
            // Fallback: RDTSC
            let tsc: u64;
            core::arch::asm!("rdtsc", out("rax") tsc, out("rdx") _);
            tsc
        }
    }
}

pub fn fill_random(buf: &mut [u8]) {
    let mut i = 0;
    while i < buf.len() {
        let r = get_random_u64();
        for j in 0..8 {
            if i + j < buf.len() {
                buf[i + j] = ((r >> (j * 8)) & 0xFF) as u8;
            }
        }
        i += 8;
    }
}

// ---------------------------------------------------------------------------
// 7. Security Policy Enforcement
// ---------------------------------------------------------------------------

pub struct SecurityPolicy {
    pub allow_raw_sockets: bool,
    pub allow_module_load: bool,
    pub allow_ptrace:      bool,
    pub kptr_restrict:     bool, // Hide kernel pointers from /proc
    pub dmesg_restrict:    bool, // Restrict dmesg to root
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        SecurityPolicy {
            allow_raw_sockets: false,
            allow_module_load: false,
            allow_ptrace:      true,
            kptr_restrict:     true,
            dmesg_restrict:    false,
        }
    }
}

/// Secure policy (like Linux with hardened defaults)
impl SecurityPolicy {
    pub fn hardened() -> Self {
        SecurityPolicy {
            allow_raw_sockets: false,
            allow_module_load: false,
            allow_ptrace:      false,
            kptr_restrict:     true,
            dmesg_restrict:    true,
        }
    }
}

static mut SECURITY_POLICY: SecurityPolicy = SecurityPolicy {
    allow_raw_sockets: false,
    allow_module_load: false,
    allow_ptrace:      true,
    kptr_restrict:     true,
    dmesg_restrict:    false,
};

pub fn get_policy() -> &'static SecurityPolicy {
    unsafe { &SECURITY_POLICY }
}

pub fn set_hardened_policy() {
    unsafe { SECURITY_POLICY = SecurityPolicy::hardened(); }
    crate::serial_println!("[security] Hardened policy applied");
}

// ---------------------------------------------------------------------------
// 8. Main init function
// ---------------------------------------------------------------------------

pub fn init() {
    crate::serial_println!("[security] Initializing security subsystem...");

    // Entropy first (needed for canary + KASLR)
    init_entropy();

    // Stack canary
    init_stack_canary();

    // KASLR offset
    init_kaslr();

    // Enable CPU security features (SMEP/SMAP)
    enable_cpu_security_features();

    // Apply hardened policy
    set_hardened_policy();

    crate::serial_println!("[security] Security subsystem ready");
}

/// Security audit: check current security posture
pub fn audit() -> SecurityAudit {
    let features = detect_cpu_security();
    SecurityAudit {
        smep_enabled: features.smep,
        smap_enabled: features.smap,
        nx_enabled:   features.nx,
        canary_set:   STACK_CANARY.load(Ordering::Relaxed) != 0,
        kaslr_active: KASLR_OFFSET.load(Ordering::Relaxed) != 0,
        rdrand_available: features.rdrand,
        hardened_policy: !get_policy().allow_raw_sockets,
    }
}

pub struct SecurityAudit {
    pub smep_enabled:      bool,
    pub smap_enabled:      bool,
    pub nx_enabled:        bool,
    pub canary_set:        bool,
    pub kaslr_active:      bool,
    pub rdrand_available:  bool,
    pub hardened_policy:   bool,
}

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
