//! `fan-files transfer` — P2P 数据传输（magic-wormhole）
//!
//! 数据方（内网）：`fan-files transfer send --dataset <name>` → 输出一次性配对码
//! 请求方（任意网络）：`fan-files transfer get <code> --output <path>` → 接收数据
//! 审计：`fan-files transfer log` → 数据方本地记录每次传输
//!
//! 机制：rendezvous 邮箱交换配对信息 → Wormhole 通道交换 UDP 打洞握手元数据 →
//! UDP 打洞直连（QUIC）→ 失败自动降级 transit relay 兜底。
//! 配对码 + 公钥识别双方身份，分块 SHA-256 校验完整性。
//!
//! ## UDP 打洞握手（自定义协议，经 Wormhole 通道）
//!
//! 发送方先发 `udp-hello`（含本方公网地址 + QUIC 证书指纹），接收方先收 2 秒：
//! 收到 `udp-hello` → 回 `udp-ack`（本方地址 + 指纹）→ 双方打洞 → QUIC 直连；
//! 收到 `transit`（旧版发送方）或超时 → 自动降级 v1 relay 路径（兼容旧版）。
//! 方向约定：**发送方 = QUIC 服务端**（quic_listen），**接收方 = QUIC 客户端**
//! （quic_connect，连发送方打洞后的真实地址）。
//!
//! 对称 NAT 支持：各端用打洞 socket 做 UDP STUN 取公网地址通告；收到对端打洞包
//! 即学到对端真实源地址（对称 NAT 下与通告地址不同）。指纹不匹配 → 中止报错，
//! 绝不降级 relay（安全问题，见规格 §七）。
//! 环境开关 `FAN_NO_UDP=1` 禁用打洞，直接走 relay。

use fan_core::config::{dirs_fan, DataLayer, Config};
use fan_core::index::sqlite::SqliteStore;
use magic_wormhole::transfer::{self, APP_CONFIG};
use magic_wormhole::transit;
use magic_wormhole::{Code, MailboxConnection, Wormhole};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 我们的 rendezvous 服务器（47.94.142.52:4000）
const RENDEZVOUS_URL: &str = "wss://hub.moilab.net/wormhole/v1";
/// 我们的 transit relay（打洞失败时中继兜底）
const TRANSIT_RELAY: &str = "tcp://47.94.142.52:4001";

fn relay_hints() -> Vec<transit::RelayHint> {
    match url::Url::parse(TRANSIT_RELAY) {
        Ok(url) => transit::RelayHint::from_urls(Some("fan-relay".to_string()), [url])
            .map(|h| vec![h])
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

// ---------- UDP 打洞握手协议（经 Wormhole 加密通道交换，KB 级） ----------

/// `FAN_NO_UDP=1` 时禁用 UDP 打洞，强制走 relay（回退测试用）。
fn udp_disabled() -> bool {
    std::env::var("FAN_NO_UDP").map(|v| v == "1").unwrap_or(false)
}

/// 握手协议消息类型
/// 相位对齐说明：wormhole 的 phase 密钥由各端自己的消息计数派生（side+count），
/// 双方各自从 0 计数、按序收发即可解密——不要求两端计数相等。
/// hello 经 rendezvous 邮箱可靠投递，必达。降级是隐式的：任一方收到对方的
/// v1 transit 消息（recv_udp_msg 解析失败）即判定对方已降级，自己同步降级。
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case", tag = "type")]
enum UdpMsg {
    /// 发送方 → 接收方：本方公网地址 + QUIC 证书指纹
    Hello {
        addr: String,
        fingerprint: String,
        nonce: u64,
    },
    /// 接收方 → 发送方：本方公网地址（回执）
    Ack {
        addr: String,
        nonce: u64,
    },
}

/// 一条直连候选（ICE-lite 风格）
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Candidate {
    kind: String, // "host" | "srflx" | "host6" | "srflx6"
    addr: String, // ip:port
    prio: u32,    // 越大越优先
}

/// 按优先级降序排序候选（大优先在前）
fn sort_candidates(v: &mut [Candidate]) {
    v.sort_by(|a, b| b.prio.cmp(&a.prio));
}

/// 本机非 loopback、非隧道接口的 IPv4 地址列表（host 候选与同网段判断共用）
fn local_ipv4_addrs() -> Vec<std::net::Ipv4Addr> {
    if_addrs::get_if_addrs().unwrap_or_default().iter()
        .filter(|i| !i.is_loopback() && !i.name.starts_with("utun"))
        .filter_map(|i| match &i.addr { if_addrs::IfAddr::V4(v4) => Some(v4.ip), _ => None })
        .collect()
}

/// 打洞成功后的直连结果（QUIC 侧用）
struct UdpDirect {
    /// 打洞 socket（交给 quinn 的 Endpoint 持有）
    sock: std::net::UdpSocket,
    /// 对端真实源地址（打洞包学习所得，对称 NAT 下 ≠ 通告地址）
    peer: std::net::SocketAddr,
}

/// 打洞成功后 QUIC 侧的传输头（双方通过 QUIC 双向流交换）
#[derive(Serialize, Deserialize, Debug, Clone)]
struct QuicHeader {
    /// 原文件名（tar 包或单文件）
    filename: String,
    /// 文件字节数
    filesize: u64,
}

/// QUIC 数据块大小（magic-wormhole v1 用 16KB；QUIC 自带流控，64KB 分块即可）
const QUIC_CHUNK: usize = 64 * 1024;

/// 获取本机公网地址（UDP STUN，与打洞同一 socket 保证端口映射一致）
fn local_public_addr(sock: &std::net::UdpSocket) -> Option<std::net::SocketAddr> {
    crate::commands::udp_punch::stun_query(sock, Duration::from_secs(2))
}

fn app_config() -> magic_wormhole::AppConfig<serde_json::Value> {
    // 显式构造（APP_CONFIG 的 app_version 字段类型固定为 transfer::AppVersion）
    // 自定义版本信息：在 abilities 里声明 udp-hole-punch 能力，供对端协商。
    // （旧版 fan-files 无此字段 → 对端判定不支持 → 自动走 v1 relay，兼容）
    // FAN_NO_UDP=1 时**不声明**该能力——否则单边禁用时，对端会等 udp-hello，
    // 而本方直接走 v1 发 transit，对端消费掉 transit 后 v1 死锁。
    let mut abilities = vec!["transfer-v1"];
    if !udp_disabled() {
        abilities.push("udp-hole-punch");
    }
    magic_wormhole::AppConfig {
        id: APP_CONFIG.id.clone(),
        rendezvous_url: std::borrow::Cow::Borrowed(RENDEZVOUS_URL),
        app_version: serde_json::json!({
            "app-version": {
                "abilities": abilities
            }
        }),
    }
}

/// 对端是否声明支持 UDP 打洞（版本消息在 wormhole connect 时已交换，无需额外消息）
fn peer_supports_udp(wormhole: &Wormhole) -> bool {
    let v = wormhole.peer_version();
    // 新版：{"app-version": {"abilities": [...]}}
    let app = v.get("app-version");
    if let Some(a) = app.and_then(|a| a.get("abilities")).and_then(|a| a.as_array()) {
        return a.iter().any(|s| s.as_str() == Some("udp-hole-punch"));
    }
    // 旧版 fan-files：直接就是 transfer::AppVersion 序列化（abilities 数组）
    if let Some(a) = v.get("abilities").and_then(|a| a.as_array()) {
        return a.iter().any(|s| s.as_str() == Some("udp-hole-punch"));
    }
    false
}

/// 打洞：在**已绑定且已做过 STUN 的 socket** 上互发打洞包。
/// 必须复用 STUN 的同一 socket——NAT 端口映射绑定在 socket 上，换 socket 端口
/// 映射就变了，通告地址立即失效（对称 NAT 双方都会向对方的"死端口"发包）。
/// 收到对端打洞包即学到对端真实源地址（可能 ≠ 通告地址），后续发包发往真实地址。
fn punch_on_socket(
    sock: std::net::UdpSocket,
    peer_msg: &UdpMsg,
    who: &str,
) -> Option<UdpDirect> {
    let (peer_addr, nonce) = match peer_msg {
        UdpMsg::Hello { addr, nonce, .. } | UdpMsg::Ack { addr, nonce } => {
            let addr: std::net::SocketAddr = addr.parse().ok()?;
            (addr, *nonce)
        }
    };
    // 对方 STUN 失败时会通告 0.0.0.0:0（占位）→ 无需打洞，立即降级
    if peer_addr.ip().is_unspecified() {
        return None;
    }
    let result = crate::commands::udp_punch::punch_establish_on_sock(
        sock,
        peer_addr,
        nonce,
        who.to_string(),
        Duration::from_secs(3),
    )?;
    Some(UdpDirect {
        sock: result.sock,
        peer: result.peer_actual,
    })
}

/// QUIC 发送端（发送方）：open_bi → 写头 → 流式发送 + SHA-256 → 等对端确认
async fn quic_send_file(
    conn: &quinn::Connection,
    path: &str,
    display_name: &str,
    progress: impl FnMut(u64, u64),
) -> Result<u64, String> {
    use futures_lite::io::AsyncReadExt;
    use sha2::Digest;
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| format!("open_bi: {e}"))?;
    let file = async_fs::File::open(path)
        .await
        .map_err(|e| format!("打开文件 {}: {}", path, e))?;
    let filesize = file
        .metadata()
        .await
        .map_err(|e| format!("文件元数据: {}", e))?
        .len();
    // 头
    let header = QuicHeader {
        filename: display_name.to_string(),
        filesize,
    };
    let header_json = serde_json::to_vec(&header).map_err(|e| format!("头序列化: {e}"))?;
    send.write_all(&(header_json.len() as u32).to_be_bytes())
        .await
        .map_err(|e| format!("写头长度: {e}"))?;
    send.write_all(&header_json)
        .await
        .map_err(|e| format!("写头: {e}"))?;
    // 数据（64KB 分块）
    let mut hasher = sha2::Sha256::new();
    let mut sent = 0u64;
    let mut progress = progress;
    progress(0, filesize);
    let mut buf = vec![0u8; QUIC_CHUNK];
    let mut reader = file;
    loop {
        let n = reader
            .read(&mut buf)
            .await
            .map_err(|e| format!("读文件: {e}"))?;
        if n == 0 {
            break;
        }
        send.write_all(&(n as u32).to_be_bytes())
            .await
            .map_err(|e| format!("写块长: {e}"))?;
        send.write_all(&buf[..n])
            .await
            .map_err(|e| format!("写块: {e}"))?;
        sent += n as u64;
        hasher.update(&buf[..n]);
        progress(sent, filesize);
    }
    // EOF 帧（len=0）
    send.write_all(&0u32.to_be_bytes())
        .await
        .map_err(|e| format!("写 EOF: {e}"))?;
    // SHA-256 结果
    let digest = hasher.finalize();
    let digest_hex = digest.iter().map(|b| format!("{:02x}", b)).collect::<String>();
    let hash_json = serde_json::to_vec(&digest_hex).map_err(|e| format!("hash 序列化: {e}"))?;
    send.write_all(&(hash_json.len() as u32).to_be_bytes())
        .await
        .map_err(|e| format!("写 hash 长: {e}"))?;
    send.write_all(&hash_json)
        .await
        .map_err(|e| format!("写 hash: {e}"))?;
    send.finish().map_err(|e| format!("finish: {e}"))?;
    // 等对端确认关闭（对端验证 SHA-256 后 close）
    let mut ack_buf = [0u8; 1];
    let _ = recv.read(&mut ack_buf).await;
    Ok(sent)
}

/// QUIC 接收端（接收方）：accept_bi → 读头 → 流式接收 + SHA-256 → 校验 → 确认
/// `output` 解析与 v1 的 get 一致：目录 → 目录/文件名；文件 → 该路径；None → ./文件名
async fn quic_recv_file(
    conn: &quinn::Connection,
    output: &Option<String>,
    progress: impl FnMut(u64, u64),
) -> Result<(String, u64), String> {
    use futures_lite::io::AsyncWriteExt;
    use sha2::Digest;
    let (mut send, mut recv) = conn
        .accept_bi()
        .await
        .map_err(|e| format!("accept_bi: {e}"))?;
    // 读头
    let len = read_frame_len(&mut recv).await?;
    if len == 0 || len > 1024 * 1024 {
        return Err(format!("头长度异常: {len}"));
    }
    let mut header_buf = vec![0u8; len];
    recv.read_exact(&mut header_buf)
        .await
        .map_err(|e| format!("读头: {e}"))?;
    let header: QuicHeader = serde_json::from_slice(&header_buf).map_err(|e| format!("解析头: {e}"))?;
    // 目标路径：--output 指定（目录→目录/文件名，文件→该路径），否则当前目录 + 原文件名
    let target_path: PathBuf = match output {
        Some(p) if PathBuf::from(p).is_dir() => PathBuf::from(p).join(&header.filename),
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(&header.filename),
    };
    if let Some(parent) = target_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut file = async_fs::File::create(&target_path)
        .await
        .map_err(|e| format!("创建文件 {}: {}", target_path.display(), e))?;
    let mut hasher = sha2::Sha256::new();
    let mut received = 0u64;
    let mut progress = progress;
    progress(0, header.filesize);
    // 数据帧（len=0 = EOF）
    let mut buf = vec![0u8; QUIC_CHUNK];
    loop {
        let n = read_frame_len(&mut recv).await?;
        if n == 0 {
            break; // EOF
        }
        if n > buf.len() {
            buf.resize(n, 0);
        }
        recv.read_exact(&mut buf[..n])
            .await
            .map_err(|e| format!("读块: {e}"))?;
        file.write_all(&buf[..n])
            .await
            .map_err(|e| format!("写文件: {e}"))?;
        received += n as u64;
        hasher.update(&buf[..n]);
        progress(received, header.filesize);
    }
    file.close().await.map_err(|e| format!("关文件: {e}"))?;
    // 读 SHA-256
    let len = read_frame_len(&mut recv).await?;
    if len == 0 || len > 1024 {
        return Err(format!("hash 长度异常: {len}"));
    }
    let mut hash_buf = vec![0u8; len];
    recv.read_exact(&mut hash_buf)
        .await
        .map_err(|e| format!("读 hash: {e}"))?;
    let peer_digest: String = serde_json::from_slice(&hash_buf).map_err(|e| format!("解析 hash: {e}"))?;
    let our_digest = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
    if peer_digest != our_digest {
        return Err(format!("SHA-256 校验失败: peer={} local={}", peer_digest, our_digest));
    }
    // 确认（close 触发对端 recv 返回）
    send.write_all(&[0x6b]).await.map_err(|e| format!("写确认: {e}"))?;
    Ok((header.filename, received))
}

/// 读一个 QUIC 帧的长度（4 字节大端）
async fn read_frame_len(recv: &mut quinn::RecvStream) -> Result<usize, String> {
    let mut b = [0u8; 4];
    recv.read_exact(&mut b)
        .await
        .map_err(|e| format!("读帧长: {e}"))?;
    Ok(u32::from_be_bytes(b) as usize)
}

/// 限时收一条 UDP 握手消息。
/// 返回 Ok(Some(msg)) = 收到；Ok(None) = 超时（对方旧版/已降级 → 走 v1）；
/// Err(parse) = 收到非 UDP 消息（如 v1 transit → 说明对方走 v1，我们降级）。
/// **注意**：调用方拿到结果后必须保持消息消费数与对端同步（见 send/get 注释）。
async fn recv_udp_msg(wormhole: &mut Wormhole, timeout: Duration, what: &str) -> Result<Option<UdpMsg>, String> {
    let res = futures_lite::future::or(
        async {
            match wormhole.receive_json::<UdpMsg>().await {
                Ok(Ok(m)) => Ok(Some(m)),
                Ok(Err(e)) => Err(format!("{what} 解析失败: {e}")),
                Err(e) => Err(format!("{what} 接收失败: {e}")),
            }
        },
        async {
            async_io::Timer::after(timeout).await;
            eprintln!("  ⏱ {what} 超时（对方可能为旧版或已降级），走 relay");
            Ok(None)
        },
    )
    .await;
    res
}

/// UDP 直连发送（发送方 = QUIC 服务端）：STUN → hello → ack → 打洞 → quic_listen → 传数据
async fn udp_send_path(
    wormhole: &mut Wormhole,
    cert: rustls_pki_types::CertificateDer<'static>,
    key: rustls_pki_types::PrivateKeyDer<'static>,
    fp: String,
    nonce: u64,
    who: &str,
    send_target: &str,
    display_name: &str,
) -> Result<u64, String> {
    // ① 绑 socket + STUN（同 socket 保证端口映射一致）→ 发 udp-hello
    //    注意：STUN 失败也要发 hello（占位 0.0.0.0:0）——接收方在等 hello，
    //    不发会让它把 v1 transit 消费掉导致协议错乱。占位地址让双方立即降级。
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind udp: {e}"))?;
    sock.set_nonblocking(true).map_err(|e| format!("set nonblocking: {e}"))?;
    let my_addr = local_public_addr(&sock);
    if my_addr.is_none() {
        eprintln!("  ⚠ UDP STUN 失败，降级 relay");
    }
    wormhole
        .send_json(&UdpMsg::Hello {
            addr: my_addr.map(|a| a.to_string()).unwrap_or_else(|| "0.0.0.0:0".into()),
            fingerprint: fp,
            nonce,
        })
        .await
        .map_err(|e| format!("send udp-hello: {e}"))?;
    let my_addr = match my_addr {
        Some(a) => a,
        None => return Err("stun-fail".into()),
    };
    // ② 等 udp-ack（超时/收到 v1 transit → 降级 relay）
    let peer_msg = match recv_udp_msg(wormhole, Duration::from_secs(15), "udp-ack").await? {
        Some(m) => m,
        None => {
            eprintln!("  ⚠ UDP 握手超时，降级 relay");
            return Err("timeout".into());
        }
    };
    let peer_hello = match &peer_msg {
        UdpMsg::Ack { .. } => peer_msg,
        UdpMsg::Hello { .. } => return Err("收到意外的 UDP 握手消息".into()),
    };
    let _ = my_addr; // 公网地址已随 hello 发出，这里仅用于 STUN 成功判定
    // ③ 打洞（复用 STUN 的同一 socket——NAT 端口映射绑定在 socket 上）
    let direct = match punch_on_socket(sock, &peer_hello, who) {
        Some(d) => d,
        None => {
            eprintln!("  ⚠ UDP 打洞失败，降级 relay");
            return Err("punch-fail".into());
        }
    };
    // ④ QUIC 服务端：在打通的 socket 上 listen，等对方连入
    let endpoint = crate::commands::quic_link::quic_listen(direct.sock, cert, key)
        .await
        .map_err(|e| format!("quic listen: {e}"))?;
    // ⑤ accept + 传数据（对方连入后 open_bi）
    let incoming = futures_lite::future::or(
        async { endpoint.accept().await.ok_or("server accept 超时".to_string()) },
        async {
            async_io::Timer::after(Duration::from_secs(15)).await;
            Err("等待对方连接超时".to_string())
        },
    )
    .await?;
    let conn = incoming.await.map_err(|e| format!("入站握手: {e}"))?;
    let sent = quic_send_file(&conn, send_target, display_name, |done, total| {
        if total > 0 {
            eprintln!("\r  进度: {}/{} ({:.0}%)", done, total, done as f64 / total as f64 * 100.0);
        }
    })
    .await?;
    drop(endpoint);
    Ok(sent)
}

/// UDP 直连接收（接收方 = QUIC 客户端）：STUN → 收 hello → ack → 打洞 → quic_connect → 传数据
async fn udp_get_path(
    wormhole: &mut Wormhole,
    hello: &UdpMsg,
    who: &str,
    output: &Option<String>,
) -> Result<(String, u64), String> {
    // ① 绑 socket + STUN（同 socket）→ 回 udp-ack
    //    注意：STUN 失败也要回 ack（占位 0.0.0.0:0）——发送方在等 ack，
    //    不回会让它等满 15s 才降级（浪费时间）。占位地址让双方立即降级。
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind udp: {e}"))?;
    sock.set_nonblocking(true).map_err(|e| format!("set nonblocking: {e}"))?;
    let my_addr = local_public_addr(&sock);
    if my_addr.is_none() {
        eprintln!("  ⚠ UDP STUN 失败，降级 relay");
    }
    let nonce = match hello {
        UdpMsg::Hello { nonce, .. } => *nonce,
        _ => return Err("收到意外的 UDP 握手消息".into()),
    };
    wormhole
        .send_json(&UdpMsg::Ack {
            addr: my_addr.map(|a| a.to_string()).unwrap_or_else(|| "0.0.0.0:0".into()),
            nonce,
        })
        .await
        .map_err(|e| format!("send udp-ack: {e}"))?;
    let _ = my_addr; // 占位/真实地址均已随 ack 发出
    // ② 打洞（复用 STUN 的同一 socket——NAT 端口映射绑定在 socket 上）
    let direct = match punch_on_socket(sock, hello, who) {
        Some(d) => d,
        None => {
            eprintln!("  ⚠ UDP 打洞失败，降级 relay");
            return Err("punch-fail".into());
        }
    };
    // ③ QUIC 客户端：连发送方打洞后的真实地址（对称 NAT 下 ≠ 通告地址）
    let expected_fp = match hello {
        UdpMsg::Hello { fingerprint, .. } => fingerprint.clone(),
        _ => unreachable!(),
    };
    let (endpoint, conn) = crate::commands::quic_link::quic_connect(
        direct.sock,
        direct.peer,
        expected_fp,
        Duration::from_secs(10),
    )
    .await
    .map_err(|e| format!("quic connect: {e}"))?;
    // ④ 接收：先读头（含文件名）→ 解析目标路径 → 写文件（与 v1 的 accept 一致）
    let (filename, received) = quic_recv_file(&conn, output, |done, total| {
        if total > 0 {
            eprintln!("\r  进度: {}/{} ({:.0}%)", done, total, done as f64 / total as f64 * 100.0);
        }
    })
    .await?;
    conn.close(0u32.into(), b"done");
    drop(endpoint);
    Ok((filename, received))
}

/// 审计日志表
const AUDIT_DDL: &str = "
CREATE TABLE IF NOT EXISTS transfer_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    direction TEXT NOT NULL,
    dataset TEXT NOT NULL,
    code TEXT NOT NULL,
    peer_key TEXT,
    bytes_sent INTEGER DEFAULT 0,
    bytes_received INTEGER DEFAULT 0,
    status TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER
);
";

pub fn run(config: &Config, layer: &DataLayer, action: TransferAction) {
    match action {
        TransferAction::Send { dataset, ttl_hours } => send(config, layer, &dataset, ttl_hours),
        TransferAction::Get { code, output } => get(config, layer, &code, output),
        TransferAction::Log { json } => log(config, layer, json),
    }
}

fn open_store(config: &Config, layer: &DataLayer) -> SqliteStore {
    let data_dir = match layer {
        DataLayer::User => dirs_fan().join("data"),
        DataLayer::Global => fan_core::config::dirs_fan_global().join("data"),
    };
    match SqliteStore::open(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("无法打开索引数据库: {}", e);
            std::process::exit(1);
        }
    }
}

fn dataset_path(store: &SqliteStore, name: &str) -> Option<String> {
    // 直接路径优先（方便分享未索引的目录）
    let p = std::path::Path::new(name);
    if p.exists() {
        return Some(p.to_string_lossy().to_string());
    }
    // 否则在索引中按名称查找
    store.all_datasets().ok()?.into_iter()
        .find(|d| d.name == name || d.path.ends_with(&format!("/{}", name)))
        .map(|d| d.path)
}

fn audit(
    store: &SqliteStore,
    direction: &str,
    dataset: &str,
    code: &str,
    peer_key: Option<&str>,
    bytes_sent: u64,
    bytes_received: u64,
    status: &str,
    started_at: i64,
) {
    let conn = match store.conn.lock() {
        Ok(c) => c,
        Err(e) => { eprintln!("审计失败: 数据库锁 {}", e); return; }
    };
    let _ = conn.execute_batch(AUDIT_DDL);
    let _ = conn.execute(
        "INSERT INTO transfer_log
         (direction, dataset, code, peer_key, bytes_sent, bytes_received, status, started_at, finished_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        rusqlite::params![direction, dataset, code, peer_key, bytes_sent as i64, bytes_received as i64, status, started_at, now_secs()],
    );
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// 发送端：`transfer send <dataset|path>`
fn send(config: &Config, layer: &DataLayer, dataset: &str, ttl_hours: u64) {
    let store = open_store(config, layer);
    let path = match dataset_path(&store, dataset) {
        Some(p) => p,
        None => {
            eprintln!("未找到 dataset '{}'。可先用 'fan-files datasets' 查看已索引的数据集。", dataset);
            std::process::exit(1);
        }
    };
    let is_dir = PathBuf::from(&path).is_dir();

    let started_at = now_secs();
    println!("fan-files transfer send");
    println!("  数据集: {}", dataset);
    println!("  路径:   {}", path);
    println!("  配对码有效期: {} 小时", ttl_hours);

    // 目录 → 临时 tar 打包（v1 协议仅支持单文件）
    let temp_tar = if is_dir {
        let tmp = std::env::temp_dir().join(format!("fan-transfer-{}.tar", now_secs()));
        match tar_directory(&path, &tmp) {
            Ok(()) => Some(tmp),
            Err(e) => { eprintln!("打包目录失败: {}", e); std::process::exit(1); }
        }
    } else { None };

    let send_target = temp_tar.as_ref().map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let display_name = if is_dir {
        format!("{}.tar", PathBuf::from(&path).file_name().unwrap_or_default().to_string_lossy())
    } else {
        PathBuf::from(&path).file_name().unwrap_or_default().to_string_lossy().to_string()
    };

    let result = async_io::block_on(async {
        let mailbox = MailboxConnection::create(app_config(), 3)
            .await
            .map_err(|e| format!("无法连接 rendezvous: {}", e))?;
        let code = mailbox.code().to_string();

        // 配对码在 create 后即生成，先打印让接收方开始连接
        println!("\n  传输码: {}", code);
        println!("  ⏳ 等待对方输入此码开始传输…（{ttl_hours}h 内有效）\n");

        let mut wormhole = Wormhole::connect(mailbox)
            .await
            .map_err(|e| format!("Wormhole 连接失败: {}", e))?;
        let peer_key = format!("{:x}", wormhole.verifier());

        // UDP 打洞优先（双方都支持才尝试；失败/禁用自动降级 relay）
        if !udp_disabled() && peer_supports_udp(&wormhole) {
            eprintln!("  🔎 对端支持 UDP 打洞，尝试直连…");
            let (cert, key, fp) = crate::commands::quic_link::gen_cert_with_fingerprint();
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0xdead_beef);
            match udp_send_path(
                &mut wormhole,
                cert,
                key,
                fp,
                nonce,
                "sender",
                send_target.as_str(),
                &display_name,
            )
            .await
            {
                Ok(sent) => {
                    println!("\n  ✅ UDP 直连传输完成，校验通过（{} 字节）", sent);
                    return Ok((code, peer_key, sent));
                }
                Err(e) => {
                    if e.contains("fingerprint") {
                        // 指纹不匹配 = 安全问题：中止，绝不降级 relay
                        return Err(format!("UDP 直连安全校验失败（指纹不匹配），中止传输: {e}"));
                    }
                    // 打洞/STUN/超时失败 → 降级 relay（消息相位已对齐，见模块注释）
                    eprintln!("  ⚠ UDP 直连失败（{}），降级 relay…", e);
                }
            }
        }

        let relay_hints = relay_hints();
        let total_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let tb = total_bytes.clone();
        let res = transfer::send_file_or_folder(
            wormhole,
            relay_hints,
            send_target.as_str(),
            display_name.clone(),
            transit::Abilities::ALL,
            |info| eprintln!("  连接: {}", fmt_conn(&info.conn_type)),
            move |done, total| {
                tb.store(total, std::sync::atomic::Ordering::Relaxed);
                if total > 0 {
                    eprintln!("\r  进度: {}/{} ({:.0}%)", done, total, done as f64 / total as f64 * 100.0);
                }
            },
            std::future::pending::<()>(),
        ).await;

        match res {
            Ok(()) => {
                let sent = total_bytes.load(std::sync::atomic::Ordering::Relaxed);
                println!("\n  ✅ 传输完成，校验通过（{} 字节）", sent);
                Ok((code, peer_key, sent))
            }
            Err(e) => Err(format!("传输失败: {}", e)),
        }
    });

    // 清理临时 tar
    if let Some(tmp) = &temp_tar {
        let _ = std::fs::remove_file(tmp);
    }

    match result {
        Ok((code, peer_key, sent)) => {
            audit(&store, "send", dataset, &code, Some(&peer_key), sent, 0, "ok", started_at);
        }
        Err(e) => {
            audit(&store, "send", dataset, "unknown", None, 0, 0, "failed", started_at);
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

/// 用系统 tar 打包目录到临时文件
fn tar_directory(src: &str, dst: &std::path::Path) -> Result<(), String> {
    let src_path = PathBuf::from(src);
    let parent = src_path.parent().unwrap_or(std::path::Path::new("."));
    let name = src_path.file_name().unwrap_or_default();
    let out = std::process::Command::new("tar")
        .arg("-cf")
        .arg(dst)
        .arg("-C")
        .arg(parent)
        .arg(name)
        .status()
        .map_err(|e| format!("tar 执行失败: {}", e))?;
    if out.success() { Ok(()) } else { Err(format!("tar 退出码 {:?}", out.code())) }
}

/// 解包 tar 到目标目录
fn extract_tar(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    let out = std::process::Command::new("tar")
        .arg("-xf")
        .arg(src)
        .arg("-C")
        .arg(dst)
        .status()
        .map_err(|e| format!("tar 解包执行失败: {}", e))?;
    if out.success() { Ok(()) } else { Err(format!("tar 解包退出码 {:?}", out.code())) }
}

/// 接收端：`transfer get <code> [--output <path>]`
fn get(config: &Config, layer: &DataLayer, code_str: &str, output: Option<String>) {
    let store = open_store(config, layer);
    let started_at = now_secs();

    let code: Code = match code_str.parse() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("无效的配对码 '{}': {}", code_str, e);
            std::process::exit(1);
        }
    };

    println!("fan-files transfer get");
    println!("  配对码: {}", code_str);
    println!("  ⏳ 正在连接…");

    let result = async_io::block_on(async {
        let mailbox = MailboxConnection::connect(app_config(), code, false)
            .await
            .map_err(|e| format!("无法连接 rendezvous（请确认配对码有效）: {}", e))?;
        let mut wormhole = Wormhole::connect(mailbox)
            .await
            .map_err(|e| format!("Wormhole 连接失败: {}", e))?;
        let peer_key = format!("{:x}", wormhole.verifier());

        // UDP 打洞优先（双方都支持才尝试；失败/禁用自动降级 relay）
        if !udp_disabled() && peer_supports_udp(&wormhole) {
            eprintln!("  🔎 对端支持 UDP 打洞，等待对方 UDP 握手…");
            // 接收方：限时等 udp-hello（旧版发送方不会发 hello，超时即降级）
            match recv_udp_msg(&mut wormhole, Duration::from_secs(15), "udp-hello").await {
                Ok(Some(hello @ UdpMsg::Hello { .. })) => {
                    // UDP 路径：目标路径在 quic_recv_file 内部解析（含文件名）
                    match udp_get_path(&mut wormhole, &hello, "receiver", &output).await {
                        Ok((file_name, received)) => {
                            println!("\n  ✅ UDP 直连接收完成，SHA-256 校验通过（{} 字节）", received);
                            println!("  已保存: {}", file_name);
                            return Ok((peer_key, received));
                        }
                        Err(e) => {
                            if e.contains("fingerprint") {
                                // 指纹不匹配 = 安全问题：中止，绝不降级 relay
                                return Err(format!(
                                    "UDP 直连安全校验失败（指纹不匹配），中止传输: {e}"
                                ));
                            }
                            eprintln!("  ⚠ UDP 直连失败（{}），降级 relay…", e);
                        }
                    }
                }
                Ok(Some(_)) | Ok(None) | Err(_) => {
                    // 对方走 v1（旧版）或已降级 → 继续 v1 路径
                }
            }
        }

        let relay_hints = relay_hints();
        let req = match transfer::request_file(wormhole, relay_hints, transit::Abilities::ALL, std::future::pending::<()>()).await {
            Ok(Some(r)) => r,
            Ok(None) => return Err("对方取消了传输".to_string()),
            Err(e) => return Err(format!("请求失败: {}", e)),
        };

        let file_name = req.file_name();
        eprintln!("  收到: {}", file_name);

        // 目标路径：--output 指定（文件或目录），否则当前目录 + 原文件名
        let target_path: PathBuf = match &output {
            Some(p) if PathBuf::from(p).is_dir() => PathBuf::from(p).join(&file_name),
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(&file_name),
        };
        if let Some(parent) = target_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // v1 accept 需要 writer（AsyncWrite）写出到文件
        let mut file = async_fs::File::create(&target_path).await
            .map_err(|e| format!("无法创建输出文件 {}: {}", target_path.display(), e))?;
        let total_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let tb = total_bytes.clone();
        let res = req.accept(
            &|info: magic_wormhole::transit::TransitInfo| eprintln!("  连接: {}", fmt_conn(&info.conn_type)),
            move |done, total| {
                tb.store(total, std::sync::atomic::Ordering::Relaxed);
                if total > 0 {
                    eprintln!("\r  进度: {}/{} ({:.0}%)", done, total, done as f64 / total as f64 * 100.0);
                }
            },
            &mut file,
            std::future::pending::<()>(),
        ).await;

        match res {
            Ok(()) => {
                let received = total_bytes.load(std::sync::atomic::Ordering::Relaxed);
                println!("\n  ✅ 接收完成，SHA-256 校验通过（{} 字节）", received);
                println!("  已保存: {}", target_path.display());
                // 若是 tar 包则自动解包到输出目录（目录数据集场景）
                if target_path.extension().map(|e| e == "tar").unwrap_or(false) {
                    let out_dir = target_path.parent().unwrap_or(std::path::Path::new("."));
                    match extract_tar(&target_path, out_dir) {
                        Ok(()) => {
                            println!("  ✅ 已解包到: {}", out_dir.display());
                            let _ = std::fs::remove_file(&target_path);
                        }
                        Err(e) => eprintln!("  ⚠ 解包失败: {}（tar 文件保留在 {}）", e, target_path.display()),
                    }
                }
                Ok((peer_key, received))
            }
            Err(e) => Err(format!("接收失败: {}", e)),
        }
    });

    match result {
        Ok((peer_key, received)) => {
            audit(&store, "get", code_str, code_str, Some(&peer_key), 0, received, "ok", started_at);
        }
        Err(e) => {
            audit(&store, "get", code_str, code_str, None, 0, 0, "failed", started_at);
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

fn fmt_conn(t: &transit::ConnectionType) -> &'static str {
    match t {
        transit::ConnectionType::Direct => "P2P 直连",
        transit::ConnectionType::Relay { .. } => "relay 中继",
        _ => "unknown",
    }
}

/// 审计：`transfer log`（--json 输出结构化记录供 GUI 使用）
fn log(config: &Config, layer: &DataLayer, json: bool) {
    let store = open_store(config, layer);
    let conn = match store.conn.lock() {
        Ok(c) => c,
        Err(e) => { eprintln!("无法读审计日志: {}", e); return; }
    };
    let _ = conn.execute_batch(AUDIT_DDL);
    let mut stmt = match conn.prepare(
        "SELECT direction, dataset, code, status, bytes_sent, bytes_received, started_at
         FROM transfer_log ORDER BY id DESC LIMIT 50"
    ) {
        Ok(s) => s,
        Err(e) => { eprintln!("查询失败: {}", e); return; }
    };
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
            r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?,
            r.get::<_, i64>(6)?,
        ))
    });
    if json {
        let mut out = Vec::new();
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                out.push(serde_json::json!({
                    "direction": row.0, "dataset": row.1, "code": row.2,
                    "status": row.3, "bytes_sent": row.4, "bytes_received": row.5,
                    "time": row.6,
                }));
            }
        }
        println!("{}", serde_json::to_string(&out).unwrap_or("[]".into()));
        return;
    }
    println!("时间                方向  数据集/码                  状态     字节");
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            let local = chrono_fmt(row.6);
            println!("{}  {:5}  {:<24}  {:7}  {}", local, row.0, row.1, row.3, row.4 + row.5);
        }
    }
}

/// 简化的本地时间格式化（无 chrono 依赖）
fn chrono_fmt(ts: i64) -> String {
    // 用系统 date 格式化（可移植）
    std::process::Command::new("date")
        .args(["-r", &ts.to_string(), "+%Y-%m-%d %H:%M:%S"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| ts.to_string())
}

/// CLI 子命令（由 main.rs 解析）
#[derive(Debug, Clone)]
pub enum TransferAction {
    Send { dataset: String, ttl_hours: u64 },
    Get { code: String, output: Option<String> },
    Log { json: bool },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_ipv4_addrs_lists_lan_ips() {
        let addrs = local_ipv4_addrs();
        assert!(!addrs.is_empty(), "应至少收集到一个 IPv4 地址");
        assert!(addrs.iter().all(|a| !a.is_loopback()), "不应包含 loopback");
    }

    #[test]
    fn candidates_sorted_by_priority() {
        let mut v = vec![
            Candidate { kind: "srflx".into(), addr: "1.2.3.4:1000".into(), prio: 20000 },
            Candidate { kind: "host".into(), addr: "10.0.0.5:1000".into(), prio: 30000 },
        ];
        sort_candidates(&mut v);
        assert_eq!(v[0].kind, "host", "host 应优先于 srflx");
    }
}
