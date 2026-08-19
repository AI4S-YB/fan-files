//! UDP 打洞：双方经 Wormhole 通道交换地址后，同时向对方公网出口发打洞包，
//! 打通 NAT 后返回可用的 UdpSocket。

use std::net::{SocketAddr, UdpSocket, ToSocketAddrs};
use std::time::Duration;

/// 打洞包：nonce 用于识别，from 标识发送方。
#[derive(Debug, Clone, PartialEq)]
pub struct PunchPacket {
    pub nonce: u64,
    pub from: String,
}

impl PunchPacket {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(8 + 4 + self.from.len());
        v.extend_from_slice(&self.nonce.to_be_bytes());
        v.extend_from_slice(&(self.from.len() as u32).to_be_bytes());
        v.extend_from_slice(self.from.as_bytes());
        v
    }
    pub fn decode(b: &[u8]) -> Result<Self, String> {
        if b.len() < 12 { return Err("too short".into()); }
        let nonce = u64::from_be_bytes(b[0..8].try_into().unwrap());
        let len = u32::from_be_bytes(b[8..12].try_into().unwrap()) as usize;
        if b.len() != 12 + len { return Err("len mismatch".into()); }
        Ok(Self { nonce, from: String::from_utf8_lossy(&b[12..]).into() })
    }
}

/// 打洞参数（规格 §四）：100ms 一发，最多 30 次 = 3 秒
pub const PUNCH_INTERVAL: Duration = Duration::from_millis(100);
pub const PUNCH_MAX_ATTEMPTS: u32 = 30;
/// 收到对方回包后再补发的拍数：先成功方立即停发会让对端一包收不到而超时，
/// 补发几拍保证对端也收到至少一拍（每 PUNCH_INTERVAL 一拍）。
pub const PUNCH_TRAILING_BEATS: u32 = 3;

/// STUN 服务器列表（按可用性排序）：用于在打洞 socket 上查询本机公网地址。
/// magic-wormhole 只支持 TCP STUN（stun.piegames.de:3478），打洞必须用 UDP STUN
/// 且必须复用打洞 socket——NAT 端口映射绑定在 socket 上，换 socket 端口就变了。
pub const STUN_SERVERS: [(&str, u16); 3] = [
    ("stun1.l.google.com", 19302),
    ("stun.cloudflare.com", 3478),
    ("stun.miwifi.com", 3478),
];

/// 打洞结果：socket + 对端实际源地址。
///
/// `peer_actual` 是对端 NAT 对本端流量的实际映射地址（从收到的打洞包源地址学习）——
/// **对称 NAT 下与对端通告的 STUN 地址不同**，QUIC 连接必须用这个地址。
#[derive(Debug)]
pub struct PunchResult {
    pub sock: UdpSocket,
    pub peer_actual: SocketAddr,
}

/// 绑一个 UDP socket，向 peer_addr 发打洞包并监听回包。
/// 返回 Some = 收到对方打洞包（打洞成功，含对端实际源地址）；None = 超时或达到 PUNCH_MAX_ATTEMPTS。
pub fn punch_establish(
    bind: SocketAddr,
    peer: SocketAddr,
    nonce: u64,
    who: String,
    timeout: Duration,
) -> Option<PunchResult> {
    let sock = UdpSocket::bind(bind).ok()?;
    punch_establish_on_sock(sock, peer, nonce, who, timeout)
}

/// 在**已绑定**的 socket 上打洞（打洞 socket 与 UDP STUN 查询必须是同一个——
/// NAT 端口映射绑定在 socket 上，换 socket 映射就变了，通告地址即失效）。
///
/// **对称 NAT 支持**：收到任意合法打洞包后，把后续发包（含补发拍）的目标切换为
/// 包的实际源地址。对称 NAT 侧的通告地址仅对 STUN 服务器有效，对端必须学会
/// "我发向它的包从哪个地址出来"才能打通。
pub fn punch_establish_on_sock(
    sock: UdpSocket,
    peer: SocketAddr,
    nonce: u64,
    who: String,
    timeout: Duration,
) -> Option<PunchResult> {
    sock.set_nonblocking(true).ok()?;
    let pkt = PunchPacket { nonce, from: who }.encode();
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = [0u8; 256];
    let mut attempts = 0u32;
    // 实际发送目标：默认对端通告地址；收到对端包后切换为对端实际源地址
    let mut target = peer;
    loop {
        attempts += 1;
        // 双上限：attempts 封顶（规格 §四：最多 30 次）+ timeout 截止
        if attempts > PUNCH_MAX_ATTEMPTS || std::time::Instant::now() >= deadline {
            return None;
        }
        // 发打洞包
        let _ = sock.send_to(&pkt, target);
        // 读回包
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                if let Ok(p) = PunchPacket::decode(&buf[..n]) {
                    if p.nonce == nonce {
                        // 对称 NAT：对端实际源地址可能与通告地址不同，后续以实际为准
                        target = from;
                        // 打洞成功：再补发几拍（每 100ms）到实际地址，保证对端也收到至少一拍
                        for _ in 0..PUNCH_TRAILING_BEATS {
                            std::thread::sleep(PUNCH_INTERVAL);
                            let _ = sock.send_to(&pkt, target);
                        }
                        return Some(PunchResult { sock, peer_actual: target });
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return None,
        }
        std::thread::sleep(PUNCH_INTERVAL);
    }
}

/// 在已绑定的 socket 上做一次 UDP STUN Binding Request，返回本机公网地址。
/// 返回 Some(公网地址) = 查询成功；None = 全部 STUN 服务器超时/失败。
///
/// **必须在打洞 socket 上查询**（同一 socket 才有同一 NAT 端口映射）。
/// socket 保持非阻塞；实现与 punch_establish 一致：轮询 + 截止时间。
pub fn stun_query(sock: &UdpSocket, timeout: Duration) -> Option<SocketAddr> {
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = [0u8; 2048];
    for (host, port) in STUN_SERVERS {
        if std::time::Instant::now() >= deadline {
            break;
        }
        let server: SocketAddr = match (host, port).to_socket_addrs().ok()?.find(|a| a.is_ipv4()) {
            Some(a) => a,
            None => continue,
        };
        // 组 STUN Binding Request：type=0x0001, len=0, magic cookie, 12B transaction id
        let tid: [u8; 12] = {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            let mut t = [0u8; 12];
            t[..8].copy_from_slice(&now.to_be_bytes());
            t
        };
        let mut req = [0u8; 20];
        req[..2].copy_from_slice(&0x0001u16.to_be_bytes());
        req[4..8].copy_from_slice(&0x2112A442u32.to_be_bytes());
        req[8..].copy_from_slice(&tid);

        let _ = sock.send_to(&req, server);
        // 等待响应（每次重发前检查，最多 1.5s/服务器）
        let server_deadline = std::time::Instant::now() + Duration::from_millis(1500);
        loop {
            if std::time::Instant::now() >= server_deadline {
                break;
            }
            match sock.recv_from(&mut buf) {
                Ok((n, _from)) => {
                    if let Some(addr) = parse_stun_response(&buf[..n], &tid) {
                        return Some(addr);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    None
}

fn parse_stun_response(data: &[u8], tid: &[u8; 12]) -> Option<SocketAddr> {
    if data.len() < 20 {
        return None;
    }
    let msg_type = u16::from_be_bytes(data[0..2].try_into().unwrap());
    let cookie = u32::from_be_bytes(data[4..8].try_into().unwrap());
    if msg_type != 0x0101 || cookie != 0x2112A442 || &data[8..20] != tid {
        return None; // 不是 Binding Success Response / 不是本请求的响应
    }
    let mut i = 20;
    while i + 4 <= data.len() {
        let attr_type = u16::from_be_bytes(data[i..i + 2].try_into().unwrap());
        let attr_len = u16::from_be_bytes(data[i + 2..i + 4].try_into().unwrap()) as usize;
        if i + 4 + attr_len > data.len() {
            return None;
        }
        let val = &data[i + 4..i + 4 + attr_len];
        // XOR-MAPPED-ADDRESS (0x0020)，IPv4
        if attr_type == 0x0020 && attr_len >= 8 && val[1] == 0x01 {
            let port = u16::from_be_bytes(val[2..4].try_into().unwrap()) ^ 0x2112;
            let ip = [
                val[4] ^ 0x21,
                val[5] ^ 0x12,
                val[6] ^ 0xA4,
                val[7] ^ 0x42,
            ];
            return Some(SocketAddr::new(std::net::IpAddr::V4(ip.into()), port));
        }
        i += 4 + ((attr_len + 3) & !3);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punch_establishes_connection_between_two_sockets() {
        // 两个本地 UDP socket（模拟双端，同机打洞必成功）
        let s1 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let s2 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        s1.set_nonblocking(true).unwrap();
        s2.set_nonblocking(true).unwrap();
        let a1 = s1.local_addr().unwrap();
        let a2 = s2.local_addr().unwrap();
        // punch_establish 会自己 bind 这两个地址；先 drop 占位 socket，否则 EADDRINUSE
        drop(s1);
        drop(s2);

        // punch_establish 是阻塞函数，用 std::thread::spawn 双线程模拟双端。
        // 机器层面已用 PUNCH_TRAILING_BEATS 补发解决“先成功方停发致对端超时”的竞态；
        // 这里再错开 100ms 启动作为冗余保险：先启动端首拍可能丢包时，
        // 重发循环 + 补发仍保证两端都收到对方至少一拍。
        let t1 = std::thread::spawn(move || {
            punch_establish(a1, a2, 42, "peer-a".into(), Duration::from_secs(2)).is_some()
        });
        std::thread::sleep(Duration::from_millis(100));
        let t2 = std::thread::spawn(move || {
            punch_establish(a2, a1, 42, "peer-b".into(), Duration::from_secs(2)).is_some()
        });
        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();
        assert!(r1, "peer-a 侧打洞应成功");
        assert!(r2, "peer-b 侧打洞应成功");
    }

    #[test]
    fn punch_returns_actual_peer_address() {
        // 对端通告地址与真实来源不同（模拟对称 NAT 场景）：
        // peer-a 绑在 a1，被告知 peer-b 在 a3（诱饵，无人监听）；
        // peer-b 真实绑在 a2，被告知 peer-a 在 a1。
        // peer-b 发包到 a1 → peer-a 收到（来源 a2）→ 学到真实地址 a2 → 改向 a2 发包。
        let s1 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let s2 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let a1 = s1.local_addr().unwrap();
        let a2 = s2.local_addr().unwrap();
        let decoy = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let a3 = decoy.local_addr().unwrap();
        drop(s1);
        drop(s2);
        drop(decoy); // 释放全部端口，让 punch_establish 绑定

        let t1 = std::thread::spawn(move || {
            punch_establish(a1, a3, 9, "peer-a".into(), Duration::from_secs(2))
        });
        std::thread::sleep(Duration::from_millis(100));
        let t2 = std::thread::spawn(move || {
            punch_establish(a2, a1, 9, "peer-b".into(), Duration::from_secs(2))
        });
        let r1 = t1.join().unwrap().expect("peer-a 打洞应成功");
        let r2 = t2.join().unwrap().expect("peer-b 打洞应成功");
        // 双方都应学到对方的真实源地址（而不是通告地址）
        assert_eq!(r1.peer_actual, a2, "peer-a 应学到 peer-b 的真实地址 a2");
        assert_eq!(r2.peer_actual, a1, "peer-b 应学到 peer-a 的真实地址 a1");
    }

    #[test]
    fn punch_times_out_when_no_peer() {
        // 对无人监听的端口打洞 → None
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let dummy = "127.0.0.1:59999".parse().unwrap();
        let addr = sock.local_addr().unwrap();
        drop(sock); // 释放端口让 punch_establish 绑定，走真实超时路径
        let result = punch_establish(addr, dummy, 1, "a".into(), Duration::from_millis(300));
        assert!(result.is_none());
    }

    /// 本机起一个假 STUN 服务器（应答 XOR-MAPPED-ADDRESS），验证 stun_query 解析。
    #[test]
    fn stun_query_parses_xor_mapped_address() {
        use std::thread;
        // 假 STUN 服务器：UDP 回包（同 socket 发回 = 源地址即应答地址）
        let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        server.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let server_addr = server.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut buf = [0u8; 64];
            match server.recv_from(&mut buf) {
                Ok((n, from)) => {
                    // 构造 Binding Success Response：type=0x0101, cookie, 同 tid
                    // XOR-MAPPED-ADDRESS：family=IPv4, port 与 IP 均被 cookie XOR
                    let port_xor = (server_addr.port() ^ 0x2112) as u16;
                    let ip_bytes = match server_addr.ip() {
                        std::net::IpAddr::V4(v4) => v4.octets(),
                        _ => [0, 0, 0, 0],
                    };
                    let mut resp = Vec::with_capacity(20 + 12);
                    resp.extend_from_slice(&0x0101u16.to_be_bytes());
                    resp.extend_from_slice(&8u16.to_be_bytes()); // attr len
                    resp.extend_from_slice(&0x2112A442u32.to_be_bytes());
                    resp.extend_from_slice(&buf[8..20]); // 回显 tid
                    resp.extend_from_slice(&0x0020u16.to_be_bytes()); // XOR-MAPPED-ADDRESS
                    resp.extend_from_slice(&8u16.to_be_bytes());
                    resp.extend_from_slice(&[0, 1]); // reserved + IPv4
                    resp.extend_from_slice(&port_xor.to_be_bytes());
                    resp.extend_from_slice(&[
                        ip_bytes[0] ^ 0x21, ip_bytes[1] ^ 0x12,
                        ip_bytes[2] ^ 0xA4, ip_bytes[3] ^ 0x42,
                    ]);
                    let _ = server.send_to(&resp, from);
                }
                Err(_) => {}
            }
        });
        // 客户端 socket 发起查询
        let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        client.set_nonblocking(true).unwrap();
        // 用假服务器地址覆盖真实 STUN 服务器列表：临时改 STUN_SERVERS 不可行（const），
        // 直接测 parse_stun_response + 组请求发给假服务器。
        // 组 Binding Request
        let tid: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let mut req = [0u8; 20];
        req[..2].copy_from_slice(&0x0001u16.to_be_bytes());
        req[4..8].copy_from_slice(&0x2112A442u32.to_be_bytes());
        req[8..].copy_from_slice(&tid);
        let _ = client.send_to(&req, server_addr);
        // 轮询读响应（客户端非阻塞，模拟 stun_query 内部逻辑）
        let mut buf = [0u8; 2048];
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let result = loop {
            match client.recv_from(&mut buf) {
                Ok((n, _)) => break parse_stun_response(&buf[..n], &tid),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break None,
            }
            if std::time::Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        handle.join().unwrap();
        assert_eq!(result, Some(server_addr), "应解析出假服务器的地址");
    }

    #[test]
    fn punch_packet_roundtrip() {
        let pkt = PunchPacket { nonce: 42, from: "sender-x".into() };
        let bytes = pkt.encode();
        let decoded = PunchPacket::decode(&bytes).unwrap();
        assert_eq!(decoded.nonce, 42);
        assert_eq!(decoded.from, "sender-x");
    }

    #[test]
    fn punch_packet_roundtrip_pins_endianness_utf8() {
        // 多字节 nonce 锁定大端字节序；CJK from 覆盖多字节 UTF-8 路径
        let pkt = PunchPacket { nonce: 0x0102030405060708, from: "发送方-甲".into() };
        let bytes = pkt.encode();
        assert_eq!(&bytes[0..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(
            u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize,
            "发送方-甲".len()
        );
        let decoded = PunchPacket::decode(&bytes).unwrap();
        assert_eq!(decoded.nonce, 0x0102030405060708);
        assert_eq!(decoded.from, "发送方-甲");
    }

    #[test]
    fn punch_packet_rejects_bad_len() {
        assert!(PunchPacket::decode(&[1,2,3]).is_err());
    }

    #[test]
    fn punch_packet_rejects_len_mismatch() {
        // 头部声明 100 字节，实际不足 → len mismatch
        let mut b = Vec::new();
        b.extend_from_slice(&0x0102030405060708u64.to_be_bytes());
        b.extend_from_slice(&100u32.to_be_bytes());
        b.extend_from_slice(b"short");
        assert!(PunchPacket::decode(&b).is_err());
        // 头部声明 0 字节，实际还有多余字节 → len mismatch
        let mut b2 = Vec::new();
        b2.extend_from_slice(&42u64.to_be_bytes());
        b2.extend_from_slice(&0u32.to_be_bytes());
        b2.extend_from_slice(b"extra");
        assert!(PunchPacket::decode(&b2).is_err());
    }
}
