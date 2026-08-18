//! UDP 打洞：双方经 Wormhole 通道交换地址后，同时向对方公网出口发打洞包，
//! 打通 NAT 后返回可用的 UdpSocket。

use std::net::{SocketAddr, UdpSocket};
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

/// 绑一个 UDP socket，向 peer_addr 发打洞包并监听回包。
/// 返回 Some(socket) = 收到对方打洞包（打洞成功）；None = 超时或达到 PUNCH_MAX_ATTEMPTS。
pub fn punch_establish(
    bind: SocketAddr,
    peer: SocketAddr,
    nonce: u64,
    who: String,
    timeout: Duration,
) -> Option<UdpSocket> {
    let sock = UdpSocket::bind(bind).ok()?;
    sock.set_nonblocking(true).ok()?;
    let pkt = PunchPacket { nonce, from: who }.encode();
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = [0u8; 256];
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        // 双上限：attempts 封顶（规格 §四：最多 30 次）+ timeout 截止
        if attempts > PUNCH_MAX_ATTEMPTS || std::time::Instant::now() >= deadline {
            return None;
        }
        // 发打洞包
        let _ = sock.send_to(&pkt, peer);
        // 读回包
        match sock.recv_from(&mut buf) {
            Ok((n, _from)) => {
                if let Ok(p) = PunchPacket::decode(&buf[..n]) {
                    if p.nonce == nonce {
                        // 打洞成功：再补发几拍（每 100ms），保证对端也收到至少一拍
                        for _ in 0..PUNCH_TRAILING_BEATS {
                            std::thread::sleep(PUNCH_INTERVAL);
                            let _ = sock.send_to(&pkt, peer);
                        }
                        return Some(sock);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return None,
        }
        std::thread::sleep(PUNCH_INTERVAL);
    }
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
    fn punch_times_out_when_no_peer() {
        // 对无人监听的端口打洞 → None
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let dummy = "127.0.0.1:59999".parse().unwrap();
        let addr = sock.local_addr().unwrap();
        drop(sock); // 释放端口让 punch_establish 绑定，走真实超时路径
        let result = punch_establish(addr, dummy, 1, "a".into(), Duration::from_millis(300));
        assert!(result.is_none());
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
