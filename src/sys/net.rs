//! Network adapters (GetIfTable2: the interface counters Task Manager's Ethernet / Wi-Fi graphs
//! use, no administrator needed) and open connections per process (GetExtendedTcpTable /
//! GetExtendedUdpTable, also no administrator). Nothing here sends a single packet.

use std::collections::HashMap;
use std::ffi::c_void;

#[link(name = "iphlpapi")]
extern "system" {
    fn GetIfTable2(table: *mut *mut u8) -> u32;
    fn FreeMibTable(p: *mut c_void);
    fn GetExtendedTcpTable(buf: *mut c_void, size: *mut u32, order: i32, af: u32, class: u32, reserved: u32) -> u32;
    fn GetExtendedUdpTable(buf: *mut c_void, size: *mut u32, order: i32, af: u32, class: u32, reserved: u32) -> u32;
}

// MIB_IF_ROW2 (1352 bytes) field offsets
const ROW2_SIZE: usize = 1352;
const OFF_LUID: usize = 0;
const OFF_ALIAS: usize = 28;
const OFF_DESC: usize = 542;
const OFF_TYPE: usize = 1128;
const OFF_FLAGS: usize = 1152;
const OFF_OPER: usize = 1156;
const OFF_TX_LINK: usize = 1192;
const OFF_RX_LINK: usize = 1200;
const OFF_IN_OCTETS: usize = 1208;
const OFF_OUT_OCTETS: usize = 1280;

const IF_TYPE_ETHERNET: u32 = 6;
const IF_TYPE_LOOPBACK: u32 = 24;
const IF_TYPE_WIFI: u32 = 71;
const IF_TYPE_WWAN: u32 = 243;
const IF_OPER_UP: u32 = 1;

const AF_INET: u32 = 2;
const AF_INET6: u32 = 23;
const TCP_TABLE_OWNER_PID_ALL: u32 = 5;
const UDP_TABLE_OWNER_PID: u32 = 1;
const TCP_STATE_LISTEN: u32 = 2;
const TCP_STATE_ESTABLISHED: u32 = 5;

#[derive(Clone, Default)]
pub struct Adapter {
    pub luid: u64,
    /// "Wi-Fi", "Ethernet"
    pub name: String,
    /// The hardware's name, e.g. "Intel(R) Wi-Fi 6E AX211"
    pub desc: String,
    /// "Wi-Fi", "Ethernet", "Cellular" or "Network"
    pub kind: &'static str,
    /// Received / sent, bytes per second
    pub rx: f64,
    pub tx: f64,
    /// Received / sent since the adapter started counting (usually boot)
    pub rx_total: u64,
    pub tx_total: u64,
    /// Link speed, bits per second
    pub link_rx: u64,
    pub link_tx: u64,
}

/// Open connections of one process
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Conns {
    /// TCP connections in the established state
    pub tcp: u32,
    /// TCP ports it listens on
    pub listen: u32,
    /// UDP sockets
    pub udp: u32,
}
impl Conns {
    pub fn total(&self) -> u32 {
        self.tcp + self.listen + self.udp
    }
}

#[derive(Default)]
pub struct NetSampler {
    prev: HashMap<u64, (u64, u64)>,
    buf: Vec<u64>,
}

unsafe fn rd<T: Copy>(row: *const u8, off: usize) -> T {
    std::ptr::read_unaligned(row.add(off) as *const T)
}

unsafe fn wstr(row: *const u8, off: usize) -> String {
    let p = row.add(off) as *const u16;
    let mut n = 0;
    while n < 256 && std::ptr::read_unaligned(p.add(n)) != 0 {
        n += 1;
    }
    let v: Vec<u16> = (0..n).map(|i| std::ptr::read_unaligned(p.add(i))).collect();
    String::from_utf16_lossy(&v)
}

impl NetSampler {
    /// Connected physical adapters (no loopback, virtual switches or filter drivers),
    /// with rates over the last `dt` seconds
    pub fn adapters(&mut self, dt: f64) -> Vec<Adapter> {
        let mut out = Vec::new();
        let mut next = HashMap::new();
        unsafe {
            let mut t: *mut u8 = std::ptr::null_mut();
            if GetIfTable2(&mut t) != 0 || t.is_null() {
                return out;
            }
            let n = rd::<u32>(t, 0) as usize;
            for i in 0..n {
                let row = t.add(8 + i * ROW2_SIZE);
                let ty: u32 = rd(row, OFF_TYPE);
                let flags: u8 = rd(row, OFF_FLAGS);
                // HardwareInterface set, FilterInterface clear
                let hardware = flags & 1 != 0 && flags & 2 == 0;
                if !hardware || ty == IF_TYPE_LOOPBACK || rd::<u32>(row, OFF_OPER) != IF_OPER_UP {
                    continue;
                }
                let luid: u64 = rd(row, OFF_LUID);
                let (rx_total, tx_total): (u64, u64) = (rd(row, OFF_IN_OCTETS), rd(row, OFF_OUT_OCTETS));
                let (rx, tx) = match self.prev.get(&luid) {
                    Some(&(pr, pt)) if dt > 0.0 => (rx_total.saturating_sub(pr) as f64 / dt, tx_total.saturating_sub(pt) as f64 / dt),
                    _ => (0.0, 0.0),
                };
                next.insert(luid, (rx_total, tx_total));
                out.push(Adapter {
                    luid,
                    name: wstr(row, OFF_ALIAS),
                    desc: wstr(row, OFF_DESC),
                    kind: match ty {
                        IF_TYPE_WIFI => "Wi-Fi",
                        IF_TYPE_ETHERNET => "Ethernet",
                        IF_TYPE_WWAN => "Cellular",
                        _ => "Network",
                    },
                    rx,
                    tx,
                    rx_total,
                    tx_total,
                    link_rx: rd(row, OFF_RX_LINK),
                    link_tx: rd(row, OFF_TX_LINK),
                });
            }
            FreeMibTable(t as _);
        }
        self.prev = next;
        // busiest first, stable by name
        out.sort_by(|a, b| (b.rx_total + b.tx_total).cmp(&(a.rx_total + a.tx_total)).then(a.name.cmp(&b.name)));
        out
    }

    /// Fills `self.buf` with one of the extended TCP/UDP tables; returns the row count
    fn table(&mut self, tcp: bool, af: u32) -> usize {
        for _ in 0..3 {
            let mut size = (self.buf.len() * 8) as u32;
            let class = if tcp { TCP_TABLE_OWNER_PID_ALL } else { UDP_TABLE_OWNER_PID };
            let p = self.buf.as_mut_ptr() as *mut c_void;
            let r = unsafe {
                if tcp {
                    GetExtendedTcpTable(p, &mut size, 0, af, class, 0)
                } else {
                    GetExtendedUdpTable(p, &mut size, 0, af, class, 0)
                }
            };
            match r {
                0 => return unsafe { rd::<u32>(self.buf.as_ptr() as *const u8, 0) } as usize,
                // ERROR_INSUFFICIENT_BUFFER: grow with some slack and retry
                122 => self.buf.resize((size as usize + 4096).div_ceil(8), 0),
                _ => return 0,
            }
        }
        0
    }

    /// Open connections per process id
    pub fn connections(&mut self) -> HashMap<u32, Conns> {
        let mut m: HashMap<u32, Conns> = HashMap::new();
        if self.buf.is_empty() {
            self.buf.resize(8192, 0);
        }
        // (tcp, family, row size, offset of state, offset of pid)
        for (tcp, af, size, st_off, pid_off) in [
            (true, AF_INET, 24, 0, 20),
            (true, AF_INET6, 56, 48, 52),
            (false, AF_INET, 12, usize::MAX, 8),
            (false, AF_INET6, 28, usize::MAX, 24),
        ] {
            let n = self.table(tcp, af);
            let base = self.buf.as_ptr() as *const u8;
            let cap = self.buf.len() * 8;
            for i in 0..n {
                let row = 4 + i * size;
                if row + size > cap {
                    break;
                }
                let pid: u32 = unsafe { rd(base, row + pid_off) };
                let e = m.entry(pid).or_default();
                if !tcp {
                    e.udp += 1;
                    continue;
                }
                match unsafe { rd::<u32>(base, row + st_off) } {
                    TCP_STATE_ESTABLISHED => e.tcp += 1,
                    TCP_STATE_LISTEN => e.listen += 1,
                    _ => {}
                }
            }
        }
        m
    }
}

/// Where a connection goes
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reach {
    Internet,
    Local,
    ThisPc,
}

/// One remote address these processes hold established TCP connections to
#[derive(Clone)]
pub struct Peer {
    pub addr: String,
    pub port: u16,
    pub reach: Reach,
    /// Connections to it
    pub count: u32,
}

/// The addresses these processes are connected to over TCP, most connections first. Read on
/// demand for the details panel (no administrator needed).
pub fn peers(pids: &[u32]) -> Vec<Peer> {
    let mut ns = NetSampler::default();
    ns.buf.resize(8192, 0);
    let mut m: HashMap<(String, u16), (Reach, u32)> = HashMap::new();
    for af in [AF_INET, AF_INET6] {
        let n = ns.table(true, af);
        let base = ns.buf.as_ptr() as *const u8;
        let cap = ns.buf.len() * 8;
        // MIB_TCPROW_OWNER_PID: state 0, remote addr 12, remote port 16, pid 20 (24 bytes)
        // MIB_TCP6ROW_OWNER_PID: remote addr 24, remote port 44, state 48, pid 52 (56 bytes)
        let (size, st_off, ra_off, rp_off, pid_off) = if af == AF_INET { (24, 0, 12, 16, 20) } else { (56, 48, 24, 44, 52) };
        for i in 0..n {
            let row = 4 + i * size;
            if row + size > cap {
                break;
            }
            unsafe {
                let pid: u32 = rd(base, row + pid_off);
                if !pids.contains(&pid) || rd::<u32>(base, row + st_off) != TCP_STATE_ESTABLISHED {
                    continue;
                }
                let port = u16::from_be(rd::<u32>(base, row + rp_off) as u16);
                let (addr, reach) = if af == AF_INET {
                    let ip: [u8; 4] = rd(base, row + ra_off);
                    (std::net::Ipv4Addr::from(ip).to_string(), reach_v4(ip))
                } else {
                    let ip: [u8; 16] = rd(base, row + ra_off);
                    let a = std::net::Ipv6Addr::from(ip);
                    match a.to_ipv4_mapped() {
                        Some(v4) => (v4.to_string(), reach_v4(v4.octets())),
                        None => {
                            let reach = if a.is_loopback() {
                                Reach::ThisPc
                            } else if (ip[0] & 0xfe) == 0xfc || (ip[0] == 0xfe && (ip[1] & 0xc0) == 0x80) {
                                Reach::Local
                            } else {
                                Reach::Internet
                            };
                            (a.to_string(), reach)
                        }
                    }
                };
                m.entry((addr, port)).or_insert((reach, 0)).1 += 1;
            }
        }
    }
    let mut v: Vec<Peer> = m.into_iter().map(|((addr, port), (reach, count))| Peer { addr, port, reach, count }).collect();
    v.sort_by(|a, b| a.reach.cmp(&b.reach).then(b.count.cmp(&a.count)).then(a.addr.cmp(&b.addr)));
    v
}

fn reach_v4(ip: [u8; 4]) -> Reach {
    if ip[0] == 127 {
        Reach::ThisPc
    } else if super::etw::is_public(ip) {
        Reach::Internet
    } else {
        Reach::Local
    }
}
