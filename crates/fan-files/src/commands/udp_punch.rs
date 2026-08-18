//! UDP 打洞：双方经 Wormhole 通道交换地址后，同时向对方公网出口发打洞包，
//! 打通 NAT 后返回可用的 UdpSocket。

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

#[cfg(test)]
mod tests {
    use super::*;

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
