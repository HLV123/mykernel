/// ARP — Address Resolution Protocol
/// Maps IPv4 → MAC addresses

use alloc::vec::Vec;
use spin::Mutex;

pub const ARP_REQUEST: u16 = 1;
pub const ARP_REPLY:   u16 = 2;

const ARP_CACHE_SIZE: usize = 16;

#[derive(Clone, Copy)]
struct ArpEntry {
    ip:  [u8; 4],
    mac: [u8; 6],
    valid: bool,
}

struct ArpCache {
    entries: [ArpEntry; ARP_CACHE_SIZE],
    next: usize,
}

impl ArpCache {
    const fn new() -> Self {
        ArpCache {
            entries: [ArpEntry { ip: [0;4], mac: [0;6], valid: false }; ARP_CACHE_SIZE],
            next: 0,
        }
    }

    fn lookup(&self, ip: &[u8; 4]) -> Option<[u8; 6]> {
        for e in &self.entries {
            if e.valid && &e.ip == ip { return Some(e.mac); }
        }
        None
    }

    fn insert(&mut self, ip: [u8; 4], mac: [u8; 6]) {
        // Update existing
        for e in self.entries.iter_mut() {
            if e.valid && e.ip == ip { e.mac = mac; return; }
        }
        // Add new
        let idx = self.next % ARP_CACHE_SIZE;
        self.entries[idx] = ArpEntry { ip, mac, valid: true };
        self.next += 1;
    }
}

static ARP_CACHE: Mutex<ArpCache> = Mutex::new(ArpCache::new());

pub fn cache_insert(ip: [u8; 4], mac: [u8; 6]) {
    ARP_CACHE.lock().insert(ip, mac);
    crate::serial_println!("[arp] cache: {}.{}.{}.{} → {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        ip[0], ip[1], ip[2], ip[3],
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
}

pub fn cache_lookup(ip: &[u8; 4]) -> Option<[u8; 6]> {
    ARP_CACHE.lock().lookup(ip)
}

/// Parse ARP packet, update cache, return reply if it's a request for our IP
pub fn process_arp(
    data: &[u8],
    our_ip: &[u8; 4],
    our_mac: &[u8; 6],
) -> Option<Vec<u8>> {
    if data.len() < 28 { return None; }

    let op = u16::from_be_bytes([data[6], data[7]]);
    let sender_mac = [data[8], data[9], data[10], data[11], data[12], data[13]];
    let sender_ip  = [data[14], data[15], data[16], data[17]];
    let target_ip  = [data[24], data[25], data[26], data[27]];

    // Learn sender
    cache_insert(sender_ip, sender_mac);

    if op == ARP_REQUEST && &target_ip == our_ip {
        crate::serial_println!("[arp] Who has {}.{}.{}.{}? reply with our MAC",
            our_ip[0], our_ip[1], our_ip[2], our_ip[3]);
        Some(build_arp_reply(*our_mac, *our_ip, sender_mac, sender_ip))
    } else {
        None
    }
}

pub fn build_arp_request(
    src_mac: [u8; 6],
    src_ip:  [u8; 4],
    target_ip: [u8; 4],
) -> Vec<u8> {
    let mut arp = Vec::with_capacity(28);
    arp.extend_from_slice(&[0x00, 0x01]); // HTYPE Ethernet
    arp.extend_from_slice(&[0x08, 0x00]); // PTYPE IPv4
    arp.push(6); arp.push(4);             // HLEN, PLEN
    arp.extend_from_slice(&[0x00, 0x01]); // OPER request
    arp.extend_from_slice(&src_mac);
    arp.extend_from_slice(&src_ip);
    arp.extend_from_slice(&[0,0,0,0,0,0]);// target MAC unknown
    arp.extend_from_slice(&target_ip);

    use crate::drivers::virtio_net::build_ethernet_frame;
    use crate::drivers::virtio_net::ETH_P_ARP;
    build_ethernet_frame([0xFF;6], src_mac, ETH_P_ARP, &arp)
}

fn build_arp_reply(
    sender_mac: [u8; 6], sender_ip: [u8; 4],
    target_mac: [u8; 6], target_ip: [u8; 4],
) -> Vec<u8> {
    let mut arp = Vec::with_capacity(28);
    arp.extend_from_slice(&[0x00, 0x01]);
    arp.extend_from_slice(&[0x08, 0x00]);
    arp.push(6); arp.push(4);
    arp.extend_from_slice(&[0x00, 0x02]); // OPER reply
    arp.extend_from_slice(&sender_mac);
    arp.extend_from_slice(&sender_ip);
    arp.extend_from_slice(&target_mac);
    arp.extend_from_slice(&target_ip);

    use crate::drivers::virtio_net::build_ethernet_frame;
    use crate::drivers::virtio_net::ETH_P_ARP;
    build_ethernet_frame(target_mac, sender_mac, ETH_P_ARP, &arp)
}
