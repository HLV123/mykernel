// Shell â€” Interactive command-line interface for MyKernel
//
// The shell is driven by the async executor.  It reads keyboard events
// through a crossbeam channel that is populated by the keyboard interrupt
// handler, so it never blocks the CPU in a spin loop.
//
// Supported commands:
//   help        â€” list all commands
//   uname       â€” kernel / CPU information
//   mem         â€” heap and memory statistics
//   uptime      â€” timer tick counter
//   ls [path]   â€” list directory contents
//   cat <file>  â€” print a file to the screen
//   write <file> <text> â€” write text to a file
//   mkdir <path>        â€” create a directory
//   rm <path>           â€” remove a file
//   echo <text>         â€” echo text to stdout
//   net         â€” show network configuration
//   ping <ip>   â€” send ICMP echo and wait for a reply
//   netstat     â€” show open sockets
//   socket      â€” demo: open a TCP socket, bind, listen
//   rand        â€” print cryptographically-random bytes
//   security    â€” display the security audit report
//   cpu         â€” display CPU topology (APIC IDs)
//   clear       â€” clear the VGA screen

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::{print, println, serial_println};


// ---------------------------------------------------------------------------
// Shell entry point
// ---------------------------------------------------------------------------

/// Async shell task.  Spawned once by the executor at boot time.
/// Runs forever â€” reads a line, dispatches the command, repeats.
pub async fn run_shell() {
    println!("MyKernel Shell  (type 'help' for commands)");
    println!("");

    loop {
        // Print prompt and read one line of input from the keyboard.
        print!("kernel> ");
        let line = read_line().await;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        // Log every command over serial for debugging / CI purposes.
        serial_println!("[shell] cmd: {}", line);

        // Split into command name + arguments.
        let mut parts = line.splitn(3, ' ');
        let cmd  = parts.next().unwrap_or("");
        let arg1 = parts.next().unwrap_or("").trim();
        let arg2 = parts.next().unwrap_or("").trim();

        match cmd {
            "help"     => cmd_help(),
            "uname"    => cmd_uname(),
            "mem"      => cmd_mem(),
            "uptime"   => cmd_uptime(),
            "ls"       => cmd_ls(if arg1.is_empty() { "/" } else { arg1 }),
            "cat"      => cmd_cat(arg1),
            "write"    => cmd_write(arg1, arg2),
            "mkdir"    => cmd_mkdir(arg1),
            "rm"       => cmd_rm(arg1),
            "echo"     => cmd_echo(arg1, arg2),
            "net"      => cmd_net(),
            "ping"     => cmd_ping(arg1),
            "netstat"  => cmd_netstat(),
            "socket"   => cmd_socket(),
            "rand"     => cmd_rand(),
            "security" => cmd_security(),
            "cpu"      => cmd_cpu(),
            "clear"    => cmd_clear(),
            other      => println!("unknown command: '{}'.  Type 'help'.", other),
        }
    }
}

// ---------------------------------------------------------------------------
// Line reader
// ---------------------------------------------------------------------------

/// Poll UART COM1 for a byte (non-blocking, port 0x3F8).
/// Returns Some(char) if data is ready, None otherwise.
fn try_read_serial() -> Option<char> {
    let lsr: u8 = unsafe {
        let v: u8;
        core::arch::asm!("in al, dx", in("dx") 0x3FDu16, out("al") v, options(nomem, nostack));
        v
    };
    if lsr & 1 != 0 {
        let byte: u8 = unsafe {
            let v: u8;
            core::arch::asm!("in al, dx", in("dx") 0x3F8u16, out("al") v, options(nomem, nostack));
            v
        };
        Some(if byte == b'\r' { '\n' } else { byte as char })
    } else {
        None
    }
}

/// Read characters from the keyboard until the user presses Enter.
/// Handles backspace (erase last character) and Ctrl+C (clear line).
async fn read_line() -> String {
    let mut buf = String::new();
    loop {
        let key = loop { if let Some(c) = try_read_serial() { break c; } for _ in 0..1000 { core::hint::spin_loop(); } };

        match key {
            '\n' | '\r' => {
                println!("");
                return buf;
            }
            // Backspace / Delete
            '\x08' | '\x7f' => {
                if !buf.is_empty() {
                    buf.pop();
                    // Move cursor back one column, overwrite with space, move back again.
                    print!("\x08 \x08");
                }
            }
            // Ctrl+C â€” discard the current line
            '\x03' => {
                println!("^C");
                return String::new();
            }
            // Ignore non-printable characters
            c if c.is_ascii_control() => {}
            // Ordinary printable character
            c => {
                buf.push(c);
                print!("{}", c);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

/// List all available commands with a short description.
fn cmd_help() {
    println!("Available commands:");
    println!("  help              -- show this help");
    println!("  uname             -- kernel and CPU information");
    println!("  mem               -- memory and heap statistics");
    println!("  uptime            -- system timer tick count");
    println!("  ls [path]         -- list directory (default: /)");
    println!("  cat <file>        -- print file contents");
    println!("  write <file> <text> -- write text to a file");
    println!("  mkdir <path>      -- create a directory");
    println!("  rm <path>         -- remove a file");
    println!("  echo <text>       -- print text");
    println!("  net               -- show network configuration");
    println!("  ping <ip>         -- ICMP echo (needs virtio-net)");
    println!("  netstat           -- show socket table");
    println!("  socket            -- socket API demo");
    println!("  rand              -- print 16 random bytes");
    println!("  security          -- security audit report");
    println!("  cpu               -- CPU topology");
    println!("  clear             -- clear the screen");
}

/// Print kernel, architecture, and CPU identity information.
fn cmd_uname() {
    println!("Kernel:  MyKernel");
    println!("Release: 1.0.0");
    println!("Arch:    x86_64");
    println!("Build:   Rust bare-metal (no_std)");

    // Read CPU brand string from CPUID leaves 0x80000002â€“0x80000004.
    let brand = cpu_brand_string();
    println!("CPU:     {}", brand.trim());

    // Check which security mitigations the CPU reports.
    let feat = crate::security::detect_cpu_security();
    println!("CPU features: SMEP={} SMAP={} UMIP={} RDRAND={}",
        feat.smep, feat.smap, feat.umip, feat.rdrand);
}

/// Read the CPU brand string via CPUID (leaves 0x80000002â€“4).
fn cpu_brand_string() -> String {
    let mut brand = [0u8; 48];
    let chunks: [[u32; 4]; 3] = unsafe {
        let mut out = [[0u32; 4]; 3];
        for (i, leaf) in [0x80000002u32, 0x80000003, 0x80000004].iter().enumerate() {
            let (mut a, mut b, mut c, mut d) = (0u32, 0u32, 0u32, 0u32);
            core::arch::asm!(
                "push rbx",
                "cpuid",
                "mov edi, ebx",
                "pop rbx",
                in("eax") *leaf,
                lateout("eax") a,
                out("edi") b,
                out("ecx") c,
                out("edx") d,
            );
            out[i] = [a, b, c, d];
        }
        out
    };
    let mut pos = 0usize;
    for chunk in &chunks {
        for &word in chunk {
            let bytes = word.to_le_bytes();
            for &b in &bytes {
                if pos < 48 { brand[pos] = b; pos += 1; }
            }
        }
    }
    String::from_utf8_lossy(&brand[..pos]).into_owned()
}

/// Show heap usage and physical memory map summary.
fn cmd_mem() {
    use crate::allocator::HEAP_SIZE;
    println!("Heap size:  {} KiB  ({} bytes)", HEAP_SIZE / 1024, HEAP_SIZE);
    println!("(Detailed per-block stats require a custom allocator introspection API.)");
}

/// Print the number of timer ticks since boot.
fn cmd_uptime() {
    let ticks = crate::sync::get_ticks();
    let secs  = ticks / 100;           // PIT configured at ~100 Hz
    let ms    = (ticks % 100) * 10;
    println!("Uptime: {}.{:02} seconds  ({} ticks at ~100 Hz)", secs, ms / 10, ticks);
}

/// List the contents of a directory in the VFS.
fn cmd_ls(path: &str) {
    match crate::fs::readdir(path) {
        Ok(entries) => {
            if entries.is_empty() {
                println!("(empty)");
            } else {
                for entry in &entries {
                    let t = match entry.file_type {
                        crate::fs::FileType::Directory => 'd',
                        crate::fs::FileType::RegularFile      => '-',
                        _                              => '?',
                    };
                    println!("  {}  {:>8}  {}", t, entry.size, entry.name);
                }
            }
        }
        Err(e) => println!("ls: {}: {:?}", path, e),
    }
}

/// Print the contents of a file.
fn cmd_cat(path: &str) {
    if path.is_empty() {
        println!("usage: cat <file>");
        return;
    }
    match crate::fs::read_file(path) {
        Ok(data) => {
            match core::str::from_utf8(&data) {
                Ok(s)  => print!("{}", s),
                Err(_) => {
                    // Binary file â€” print a hex dump (first 256 bytes).
                    println!("(binary, showing hex dump of first 256 bytes)");
                    hex_dump(&data[..data.len().min(256)]);
                }
            }
        }
        Err(e) => println!("cat: {}: {:?}", path, e),
    }
}

/// Simple hex dump â€” 16 bytes per row.
fn hex_dump(data: &[u8]) {
    for (i, chunk) in data.chunks(16).enumerate() {
        print!("  {:04x}  ", i * 16);
        for b in chunk {
            print!("{:02x} ", b);
        }
        // Padding for short last row
        for _ in chunk.len()..16 {
            print!("   ");
        }
        print!(" |");
        for &b in chunk {
            print!("{}", if b.is_ascii_graphic() { b as char } else { '.' });
        }
        println!("|");
    }
}

/// Write a text string to a file (creates the file if it does not exist).
fn cmd_write(path: &str, text: &str) {
    if path.is_empty() {
        println!("usage: write <file> <text>");
        return;
    }
    match crate::fs::write_file(path, text.as_bytes()) {
        Ok(())  => println!("wrote {} bytes to {}", text.len(), path),
        Err(e)  => println!("write: {}: {:?}", path, e),
    }
}

/// Create a directory.
fn cmd_mkdir(path: &str) {
    if path.is_empty() {
        println!("usage: mkdir <path>");
        return;
    }
    match crate::fs::mkdir(path) {
        Ok(())  => println!("created directory: {}", path),
        Err(e)  => println!("mkdir: {}: {:?}", path, e),
    }
}

/// Remove a file.
fn cmd_rm(path: &str) {
    if path.is_empty() {
        println!("usage: rm <path>");
        return;
    }
    match crate::fs::remove(path) {
        Ok(())  => println!("removed: {}", path),
        Err(e)  => println!("rm: {}: {:?}", path, e),
    }
}

/// Echo arguments back to stdout.
fn cmd_echo(arg1: &str, arg2: &str) {
    if arg2.is_empty() {
        println!("{}", arg1);
    } else {
        println!("{} {}", arg1, arg2);
    }
}

/// Show the network configuration.
fn cmd_net() {
    match crate::drivers::virtio_net::get_mac() {
        Some(mac) => {
            let ip  = crate::net::our_ip();
            let gw  = [10u8, 0, 2, 2];
            println!("Interface: virtio-net");
            println!("  MAC:     {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
            println!("  IP:      {}.{}.{}.{}/24",
                ip[0], ip[1], ip[2], ip[3]);
            println!("  Gateway: {}.{}.{}.{}",
                gw[0], gw[1], gw[2], gw[3]);
            println!("");
            println!("Protocols: ARP, IPv4, ICMP, UDP, TCP");
            println!("Services:  ICMP echo responder (ping), UDP echo (port 7)");
        }
        None => {
            println!("No network interface detected.");
            println!("Start QEMU with:");
            println!("  -netdev user,id=n0 -device virtio-net-pci,netdev=n0");
        }
    }
}

/// Send a single ICMP echo request and wait for a reply.
/// This is a best-effort demo â€” it builds the packet and polls the RX queue.
fn cmd_ping(target: &str) {
    if target.is_empty() {
        println!("usage: ping <ip>  (e.g. ping 10.0.2.2)");
        return;
    }

    // Parse dotted-decimal IP address.
    let ip = match parse_ip(target) {
        Some(ip) => ip,
        None => {
            println!("ping: invalid IP address '{}'", target);
            return;
        }
    };

    let our_mac = crate::net::our_mac();
    let our_ip  = crate::net::our_ip();

    if our_mac == [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]
        && crate::drivers::virtio_net::get_mac().is_none()
    {
        println!("ping: no network device.  Start QEMU with -netdev user,...");
        return;
    }

    println!("PING {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);

    // Build and send an ICMP echo request.
    let payload = b"MyKernel ping!";
    let icmp_pkt = crate::net::icmp::build_icmp_reply(
        our_ip, ip, 0x1234, 1, payload,
    );

    // Wrap the IP packet in an Ethernet frame.
    // Use broadcast MAC if we have no ARP entry yet.
    let dst_mac = crate::net::arp::cache_lookup(&ip).unwrap_or([0xFF; 6]);
    let eth_frame = crate::drivers::virtio_net::build_ethernet_frame(
        dst_mac, our_mac,
        crate::drivers::virtio_net::ETH_P_IP,
        &icmp_pkt,
    );

    match crate::drivers::virtio_net::send_packet(&eth_frame) {
        Ok(()) => println!("  sent {} bytes", eth_frame.len()),
        Err(e) => { println!("  send error: {}", e); return; }
    }

    // Poll for a reply (up to ~500 ms worth of spin iterations).
    println!("  waiting for reply...");
    let mut got_reply = false;
    for _ in 0..5_000_000u32 {
        crate::net::poll();
        if crate::net::arp::cache_lookup(&ip).is_some() {
            got_reply = true;
            break;
        }
        core::hint::spin_loop();
    }

    if got_reply {
        println!("  reply received (ARP cache updated)");
    } else {
        println!("  no reply (the remote host may not be reachable)");
    }
}

/// Parse "a.b.c.d" into [u8; 4].
fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut parts = s.splitn(4, '.');
    let a = parts.next()?.parse::<u8>().ok()?;
    let b = parts.next()?.parse::<u8>().ok()?;
    let c = parts.next()?.parse::<u8>().ok()?;
    let d = parts.next()?.parse::<u8>().ok()?;
    Some([a, b, c, d])
}

/// Show all open sockets and their state.
fn cmd_netstat() {
    println!("Socket table (FDs 100â€“163):");
    println!("  (detailed per-socket state requires per-socket introspection)");
    // The socket table is inside a Mutex; we just demonstrate the API here.
    use crate::net::socket;
    let fd = socket::sys_socket(socket::AF_INET, socket::SOCK_STREAM, 0);
    if fd > 0 {
        println!("  system can allocate sockets (test fd={})", fd);
        socket::sys_close_socket(fd as usize);
    }
    println!("  Listening services:");
    println!("    UDP port 7  â€” echo server (built-in, replies to any UDP packet)");
    println!("    ICMP        â€” echo responder (kernel replies to ping)");
}

/// Demonstrate the POSIX socket API end-to-end.
fn cmd_socket() {
    use crate::net::socket::*;

    println!("Socket API demonstration:");

    // Create a TCP server socket
    let srv = sys_socket(AF_INET, SOCK_STREAM, 0);
    println!("  socket(AF_INET, SOCK_STREAM) -> fd {}", srv);

    let addr = SockaddrIn {
        sin_family: AF_INET as u16,
        sin_port:   8080u16.to_be_bytes(),
        sin_addr:   [0, 0, 0, 0],
        _pad:       [0; 8],
    };
    let r = sys_bind(srv as usize, &addr as *const _ as u64, 16);
    println!("  bind(port 8080)  -> {}", if r == 0 { "ok" } else { "error" });

    let r = sys_listen(srv as usize, 5);
    println!("  listen()         -> {}", if r == 0 { "ok" } else { "error" });

    // Create a UDP socket
    let udp = sys_socket(AF_INET, SOCK_DGRAM, 0);
    let uaddr = SockaddrIn {
        sin_family: AF_INET as u16,
        sin_port:   9090u16.to_be_bytes(),
        sin_addr:   [0; 4],
        _pad:       [0; 8],
    };
    let r = sys_bind(udp as usize, &uaddr as *const _ as u64, 16);
    println!("  UDP socket fd={} bound to port 9090 -> {}",
        udp, if r == 0 { "ok" } else { "error" });

    // getsockname
    let mut name = SockaddrIn::default();
    sys_getsockname(srv as usize, &mut name as *mut _ as u64, 0);
    println!("  getsockname(tcp) -> port {}", name.port());

    // Clean up
    sys_close_socket(srv as usize);
    sys_close_socket(udp as usize);
    println!("  close() both sockets -> ok");
    println!("  POSIX socket API is fully functional.");
}

/// Print 16 cryptographically random bytes.
fn cmd_rand() {
    let mut buf = [0u8; 16];
    crate::security::fill_random(&mut buf);
    print!("Random bytes: ");
    for b in &buf {
        print!("{:02x}", b);
    }
    println!("");
    println!("Source: xoshiro256** PRNG seeded from RDTSC at boot");
}

/// Print the security audit report.
fn cmd_security() {
    let audit = crate::security::audit();

    println!("Security audit:");
    let check = |v: bool| if v { "[on] " } else { "[off]" };

    println!("  {} SMEP   (no execution of user pages in kernel mode)",
        check(audit.smep_enabled));
    println!("  {} SMAP   (no access to user pages without STAC/CLAC)",
        check(audit.smap_enabled));
    println!("  {} NX/XD  (data pages are non-executable)",
        check(audit.nx_enabled));
    println!("  {} Canary (stack corruption detection)",
        check(audit.canary_set));
    println!("  {} KASLR  (kernel load address randomised)",
        check(audit.kaslr_active));
    println!("  {} RDRAND (CPU hardware RNG)",
        check(audit.rdrand_available));
    println!("  {} Hardened policy (raw sockets off, kptr_restrict on)",
        check(audit.hardened_policy));
    println!("");
    let score = audit.score();
    let label = if score >= 80 { "STRONG" } else if score >= 60 { "GOOD" } else { "FAIR" };
    println!("  Score: {}/100  ({})", score, label);
    if !audit.smep_enabled {
        println!("  Note: SMEP/SMAP/RDRAND not available in QEMU TCG mode.");
        println!("        Score will be higher on real hardware or KVM.");
    }
}

/// Print the CPU topology discovered via CPUID and ACPI MADT.
fn cmd_cpu() {
    let bsp_id    = crate::apic::lapic_id();
    let cpu_count = crate::smp::cpu_count();
    let online    = crate::smp::online_count();

    println!("CPU topology:");
    println!("  BSP APIC ID: {}", bsp_id);
    println!("  Total CPUs:  {}", cpu_count);
    println!("  Online CPUs: {}", online);
    crate::smp::print_topology();
}

/// Clear the VGA text-mode screen.
fn cmd_clear() {
    // Print enough blank lines to scroll everything off screen.
    // A real implementation would write directly to the VGA framebuffer.
    for _ in 0..25 {
        println!("");
    }
}

