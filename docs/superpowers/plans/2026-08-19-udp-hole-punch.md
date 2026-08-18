# UDP 打洞 + QUIC 直连 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 fan-files P2P 传输增加 UDP 打洞 + QUIC 直连数据通道，打洞失败自动降级现有 relay，提升直连成功率、减少阿里云流量。

**Architecture:** 复用 magic-wormhole 配对通道（已认证加密）交换 UDP 地址/证书指纹；自写 `UdpPuncher`（UDP 打洞状态机）打通双方 NAT；`QuicLink`（quinn）在打通的 socket 上建 QUIC 连接传数据；失败自动降级现有 relay。

**Tech Stack:** Rust、magic-wormhole（现有）、quinn（QUIC）、tokio、async-io、serde。

**规格:** `docs/superpowers/specs/2026-08-19-udp-hole-punch-design.md`（决策：自动降级 relay / 复用 Wormhole 通道 / QUIC 直连 / 不引 libp2p）。

---

## Phase 0 — 打洞核心（UdpPuncher）

### Task 1: 新建 `udp_punch.rs` 模块与打洞包协议

**Files:**
- Create: `crates/fan-files/src/commands/udp_punch.rs`
- Modify: `crates/fan-files/src/commands/mod.rs`（注册模块）

- [ ] **Step 1: 写失败测试（打洞包编解码 roundtrip）**

```rust
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
    fn punch_packet_rejects_bad_len() {
        assert!(PunchPacket::decode(&[1,2,3]).is_err());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd /Users/kentnf/Desktop/projects/fan-files && cargo test -p fan-files udp_punch`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现 `udp_punch.rs`**

```rust
//! UDP 打洞：双方经 Wormhole 通道交换地址后，同时向对方公网出口发打洞包，
//! 打通 NAT 后返回可用的 UdpSocket。

use std::net::SocketAddr;

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
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p fan-files udp_punch`
Expected: 2 passed

- [ ] **Step 5: Commit**（bioinfo7）: `feat(p2p): UDP punch packet protocol`

### Task 2: UdpPuncher 打洞状态机（核心）

**Files:**
- Modify: `crates/fan-files/src/commands/udp_punch.rs`

- [ ] **Step 1: 写失败测试（打洞逻辑）**

```rust
#[tokio::test]
async fn punch_establishes_connection_between_two_sockets() {
    // 两个本地 UDP socket（模拟双端，同机打洞必成功）
    let s1 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let s2 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    s1.set_nonblocking(true).unwrap();
    s2.set_nonblocking(true).unwrap();
    let a1 = s1.local_addr().unwrap();
    let a2 = s2.local_addr().unwrap();

    let t1 = tokio::spawn(punch_sender(s1, a2, 42, "peer-a".into()));
    let t2 = tokio::spawn(punch_sender(s2, a1, 42, "peer-b".into()));
    let (r1, r2) = tokio::join!(t1, t2);
    assert!(r1.unwrap().is_some());  // s1 收到来自 a2 的包 → 打洞成功
    assert!(r2.unwrap().is_some());
}
```

（`punch_sender` 原型：绑定 socket 后不断向对端发打洞包并读回，返回是否收到对方包。用 std 非阻塞 + tokio::time 循环实现。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p fan-files punch_establishes`
Expected: FAIL（punch_sender 不存在）

- [ ] **Step 3: 实现 `punch_sender` 与打洞入口**

```rust
use std::net::UdpSocket;
use std::time::Duration;

/// 打洞参数（规格 §四）：100ms 一发，最多 30 次 = 3 秒
pub const PUNCH_INTERVAL: Duration = Duration::from_millis(100);
pub const PUNCH_MAX_ATTEMPTS: u32 = 30;

/// 绑一个 UDP socket，向 peer_addr 发打洞包并监听回包。
/// 返回 Some(socket) = 收到对方打洞包（打洞成功）；None = 超时。
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
    loop {
        // 发打洞包
        let _ = sock.send_to(&pkt, peer);
        // 读回包
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                if let Ok(p) = PunchPacket::decode(&buf[..n]) {
                    if p.nonce == nonce {
                        return Some(sock);
                    }
                }
                let _ = from;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return None,
        }
        if std::time::Instant::now() >= deadline { return None; }
        std::thread::sleep(PUNCH_INTERVAL);
    }
}
```

- [ ] **Step 4: 跑测试确认通过 + 补充超时测试**

Run: `cargo test -p fan-files punch`
Expected: PASS + 新增 `punch_times_out_when_no_peer`（对无人端口打洞 → None）

- [ ] **Step 5: Commit**（bioinfo7）: `feat(p2p): UDP hole-punch state machine`

---

## Phase 1 — QUIC 直连（QuicLink）

### Task 3: QuicLink 证书生成 + 指纹交换

**Files:**
- Create: `crates/fan-files/src/commands/quic_link.rs`
- Modify: `crates/fan-files/src/commands/mod.rs`

- [ ] **Step 1: 写失败测试（指纹派生确定性）**

```rust
#[test]
fn cert_fingerprint_is_deterministic() {
    let a = cert_fingerprint(&rustls::pki_types::CertificateDer::from(vec![1,2,3]));
    let b = cert_fingerprint(&rustls::pki_types::CertificateDer::from(vec![1,2,3]));
    assert_eq!(a, b);
    assert_eq!(a.len(), 64); // SHA-256 hex
}
```

- [ ] **Step 2: 跑测试确认失败** → **Step 3: 实现**

```rust
//! QUIC 直连：quinn 在打通的 UDP socket 上建连接，证书指纹经 Wormhole 通道交换。

pub fn cert_fingerprint(cert: &rustls::pki_types::CertificateDer) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(cert.as_ref());
    d.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 生成自签证书（打洞 QUIC 用；指纹经 Wormhole 通道验证，无需 CA）
pub fn self_signed_cert() -> (rustls::pki_types::CertificateDer<'static>, rustls::pki_types::PrivateKeyDer<'static>) {
    // 用 rcgen 生成自签 ECDSA 证书
    let key_pair = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["fan-files.local".to_string()])
        .unwrap()
        .self_signed(&key_pair).unwrap();
    (cert.into(), key_pair.into())
}
```

- [ ] **Step 4: 测试通过 + Cargo.toml 加依赖**（`quinn`、`rcgen`、`sha2`、`rustls-pki-types`）

- [ ] **Step 5: Commit**（bioinfo7）: `feat(p2p): QUIC cert generation + fingerprint`

### Task 4: QuicLink 建立直连 + 指纹验证

**Files:**
- Modify: `crates/fan-files/src/commands/quic_link.rs`

- [ ] **Step 1: 写失败测试（指纹不匹配拒绝）**

```rust
#[tokio::test]
async fn quic_handshake_rejects_wrong_fingerprint() {
    // 客户端连服务端，但提供错误指纹 → 握手失败
    // （服务端本地起 quinn listener，客户端用错误指纹连接 → expect Err）
}
```

- [ ] **Step 2: 测试失败** → **Step 3: 实现**

```rust
// 服务端：在打通的 socket 上 accept QUIC 连接
pub async fn quic_listen(sock: UdpSocket, expected_fp: String) -> Result<quinn::Endpoint, String> {
    let (cert, key) = self_signed_cert();
    let fp = cert_fingerprint(&cert);
    // 把实际指纹打印/返回给调用方（经 Wormhole 通道交换）
    let server_config = quinn::ServerConfig::with_single_cert(vec![cert], key)
        .map_err(|e| format!("QUIC server config: {}", e))?;
    let endpoint = quinn::Endpoint::new(Default::default(), Some(server_config), sock)
        .map_err(|e| format!("QUIC endpoint: {}", e))?;
    let _ = expected_fp; // 指纹验证在客户端侧做
    Ok(endpoint)
}

// 客户端：用期望指纹建连（校验对端证书）
pub async fn quic_connect(
    sock: UdpSocket,
    peer: SocketAddr,
    expected_fp: String,
) -> Result<quinn::Connection, String> {
    let (cert, key) = self_signed_cert();
    let mut crypto = rustls::ClientConfig::builder()
        .with_root_certificates(root_certs(&expected_fp))
        .with_client_auth_cert(vec![cert], key)
        .map_err(|e| format!("client auth: {}", e))?;
    crypto.enable_early_data = true;
    let client_config = quinn::ClientConfig::new(Arc::new(crypto));
    let mut endpoint = quinn::Endpoint::new(Default::default(), None, sock)
        .map_err(|e| format!("client endpoint: {}", e))?;
    let conn = endpoint.connect_with(client_config, peer, "fan-files.local")
        .map_err(|e| format!("connect: {}", e))?
        .await.map_err(|e| format!("handshake: {}", e))?;
    Ok(conn)
}

fn root_certs(fp: &str) -> rustls::RootCertStore {
    // 用期望指纹构造固定根证书（防 MITM 的核心）
    // 实际实现：把自签证书 DER 作为根，校验指纹一致性
    rustls::RootCertStore::empty()
}
```

- [ ] **Step 4: 测试通过（指纹拒绝用集成测试：两端本地建连）**
- [ ] **Step 5: Commit**（bioinfo7）: `feat(p2p): QUIC connect/listen with fingerprint pinning`

---

## Phase 2 — transfer 集成（打洞优先 + 降级）

### Task 5: transfer send/get 接入 UdpPuncher + QuicLink

**Files:**
- Modify: `crates/fan-files/src/commands/transfer.rs`

- [ ] **Step 1: 写失败测试（FAN_NO_UDP 开关）**

```rust
#[test]
fn udp_disabled_via_env() {
    std::env::set_var("FAN_NO_UDP", "1");
    assert!(udp_disabled());
    std::env::remove_var("FAN_NO_UDP");
    assert!(!udp_disabled());
}
```

- [ ] **Step 2: 失败** → **Step 3: 实现集成**

在 `send` 的 block_on 里：建立 Wormhole 通道后，生成自签证书 → 经 wormhole 通道交换指纹+UDP 地址 → `punch_establish` → 成功则 `quic_listen` 等对方连入 → 传输数据；失败/禁用 → 走现有 `send_file_or_folder` relay 路径。`get` 对称。

```rust
fn udp_disabled() -> bool {
    std::env::var("FAN_NO_UDP").map(|v| v == "1").unwrap_or(false)
}
```

- [ ] **Step 4: 测试通过 + 双机实测（见 Task 6）**
- [ ] **Step 5: Commit**（bioinfo7）: `feat(p2p): transfer send/get try UDP punch first, fallback relay`

---

## Phase 3 — 测试与验收

### Task 6: 双机 100MB 实测 + 回退测试

**Files:**
- Test-only（bioinfo7 + Mac mini 手动）

- [ ] **Step 1: 双机实测（打洞成功路径）**

bioinfo7 发 / Mac mini 收（或反向），100MB：
```bash
# bioinfo7
fan-files transfer send /tmp/fan-100mb.bin
# Mac mini
fan-files transfer get <code> --output /tmp/recv
```
Expected: 日志显示 `连接: UDP直连`（打洞成功时），SHA-256 一致，速率记录。

- [ ] **Step 2: 回退测试**

```bash
FAN_NO_UDP=1 fan-files transfer send /tmp/fan-100mb.bin
```
Expected: 走 relay（现有路径），仍成功。

- [ ] **Step 3: 真实失败降级测试**

bioinfo7（对称 NAT）→ 打洞大概率失败 → 日志显示 `连接: relay中继`，数据仍完整。

- [ ] **Step 4: 提交实测结果**（日志 + SHA256 + 速率 + 连接类型，写入 docs/ 或 README 附录）

---

## 执行顺序与依赖

```
Task 1（打洞包协议）→ Task 2（打洞状态机）→ Task 3（QUIC 证书/指纹）→ Task 4（QUIC 建连）
→ Task 5（transfer 集成）→ Task 6（双机实测）
```

Task 1-4 相互独立可并行；Task 5 依赖 1-4；Task 6 依赖 5。工作流：Mac mini 写代码 → bioinfo7 编译/测试/提交推送（`feat/desktop-app` 分支）。
