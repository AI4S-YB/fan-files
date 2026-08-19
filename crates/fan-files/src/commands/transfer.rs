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
//! 发送方先发 `udp-hello`（候选列表 + QUIC 证书指纹），接收方先收 2 秒：
//! 收到 `udp-hello` → 回 `udp-ack`（候选列表）→ 双方打洞 → QUIC 直连；
//! 收到 `transit`（旧版发送方）或超时 → 自动降级 v1 relay 路径（兼容旧版）。
//! 方向约定：**发送方 = QUIC 服务端**（quic_listen），**接收方 = QUIC 客户端**
//! （quic_connect，连发送方打洞后的真实地址）。
//!
//! 对称 NAT 支持：各端用打洞 socket 做 UDP STUN 取公网地址，作为 srflx 候选通告；
//! 收到对端打洞包即学到对端真实源地址（对称 NAT 下与通告地址不同）。指纹不匹配
//! → 中止报错，绝不降级 relay（安全问题，见规格 §七）。
//! 环境开关 `FAN_NO_UDP=1` 禁用打洞相位，直接走 relay（双方仍声明 v3 能力，
//! 走 relay 多线程分块路径，见下）。
//!
//! ## 分块续传（能力 v3：FileMeta/ChunkStatus）
//!
//! QUIC 建连后，双方经 wormhole 交换分块头：发送方发 `file-meta`（文件名、大小、
//! 文件级 SHA-256、块大小/块数），接收方查本地清单（`.fan-files/partial/<hash16>.
//! chunks.json`）回 `chunk-status`（已完成块集合），发送方只传缺失块 → 断线续传。
//! 完成后接收方做文件级 SHA-256 校验，通过才 rename 到目标路径并清理清单。
//! 清单不匹配（文件已变化）→ 空回执 = 全量传输。
//! 发送方文件级 SHA-256 在后台线程计算（与打洞/accept 并行）——hello 先于哈希发出，
//! 大文件哈希不再挤占接收方 15s udp-hello 窗口；FileMeta 等哈希就绪后再发（此时
//! 哈希通常早已完成），哈希失败/超时 → 本端降级 relay，绝不迟到污染对方消息流。
//!
//! ## relay 多线程分块（Task 5：能力 v3 新增 RelayChunk 消息）
//!
//! UDP 直连失败（或 `FAN_NO_UDP=1`）降级 relay 时，v3 对端之间不再走 v1 单流
//! 整文件传输，而是 v3 relay 分块：发送方发 `file-meta` → 收 `chunk-status` →
//! 对每个缺失块新建**独立** magic-wormhole 会话（`MailboxConnection::create`
//! 生成块配对码），经主通道发 `relay-chunk {code, index}` 告知接收方 → 接收方
//! 用该码发起 `transfer get` 收块（OffsetWriter 写 partial 对应 offset，逐块
//! 原子更新清单）。发送方 `concurrency` 个 worker 并行（每 worker 一个会话），
//! 失败重试 ≤3 次（重试 = 新会话 + 新码）；块级 SHA-256 由 magic-wormhole v1
//! 协议自带校验。partial + 清单与 QUIC 路径直接复用——relay 只传缺失块。
//! 消息相位：UDP 相位各端收发计数对称（见 UdpMsg 注释），降级后
//! FileMeta/ChunkStatus/RelayChunk 按序对齐，与 v1 降级同规则。

use crate::commands::chunked;
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
    /// 发送方 → 接收方：候选列表 + QUIC 证书指纹
    Hello {
        candidates: Vec<Candidate>,
        fingerprint: String,
        nonce: u64,
    },
    /// 接收方 → 发送方：候选列表（回执）
    Ack {
        candidates: Vec<Candidate>,
        nonce: u64,
    },
    /// 接收方 → 发送方：所有候选尝试失败，放弃 UDP 直连。
    /// 发送方收到后立即降级 relay（不等 accept 超时 15s）。
    /// 相位对称：双方各多收发 1 条消息，v1 降级仍对齐。
    Abort,
    /// 发送方 → 接收方：文件元数据（分块传输头，在 QUIC 建连后、数据流之前交换）。
    /// sha256 = 文件级 SHA-256 hex（前 16 字符作清单键 hash16）。
    FileMeta {
        name: String,
        size: u64,
        sha256: String,
        chunk_size: u64,
        chunk_count: u32,
    },
    /// 接收方 → 发送方：已完成块集合（清单回执；空 = 全量传输）
    ChunkStatus { done: Vec<u32> },
    /// 发送方 → 接收方：relay 块配对码（块 index 对应的**独立** magic-wormhole
    /// 会话码）。接收方用该码发起 `transfer get` 收块；块级 SHA-256 由
    /// magic-wormhole v1 协议自带校验。发送方按缺失块逐块发送（并发 ≤4 个
    /// 在途），失败重试 = 新会话 + 新码（重发 RelayChunk）。
    RelayChunk { code: String, index: u32 },
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

/// 多流并发传块的 worker 流数（每流一块 4MB 块）
const QUIC_CHUNK_CONCURRENCY: usize = 4;

/// 获取本机公网地址（UDP STUN，与打洞同一 socket 保证端口映射一致）
fn local_public_addr(sock: &std::net::UdpSocket) -> Option<std::net::SocketAddr> {
    crate::commands::udp_punch::stun_query(sock, Duration::from_secs(2))
}

fn app_config() -> magic_wormhole::AppConfig<serde_json::Value> {
    // 显式构造（APP_CONFIG 的 app_version 字段类型固定为 transfer::AppVersion）
    // 自定义版本信息：在 abilities 里声明 udp-hole-punch 能力，供对端协商。
    // （旧版 fan-files 无此字段 → 对端判定不支持 → 自动走 v1 relay，兼容）
    // 能力名带协议版本：UdpMsg v3（候选列表 + FileMeta/ChunkStatus 分块续传 +
    // RelayChunk relay 分块）与 v2/v1 互不兼容，用不同能力名让新旧端在协商层
    // 即区分：新端发 v3 hello 给旧端（v2 能力名不匹配）→ peer_supports_udp 为
    // false → 双方干净地走 v1 relay（协商层隔离，无相位错乱）。
    // **始终声明 v3**（含 FAN_NO_UDP=1）：该开关只跳过 UDP 打洞相位，双方仍
    // 走 v3 relay 分块——消息流一致（FileMeta/ChunkStatus/RelayChunk，见
    // send/get 的降级分支），单边禁用时对端在 hello 窗口收到 FileMeta 即进入
    // relay 分块，不再有"本方走 v1 发 transit、对端等 udp-hello"的 v1 死锁。
    let abilities = vec!["transfer-v1", "udp-hole-punch-v3"];
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

/// 对端是否声明支持 UDP 打洞 v3（候选列表 + FileMeta/ChunkStatus 分块续传）
/// 版本消息在 wormhole connect 时已交换，无需额外消息。
/// **能力名带版本**：v1（单 addr）/ v2（候选列表）与 v3（分块续传）互不兼容，
/// 必须精确匹配 "udp-hole-punch-v3"——旧端能力名不匹配 → false → 走 v1 relay。
fn peer_supports_udp(wormhole: &Wormhole) -> bool {
    let v = wormhole.peer_version();
    // 新版：{"app-version": {"abilities": [...]}}
    let app = v.get("app-version");
    if let Some(a) = app.and_then(|a| a.get("abilities")).and_then(|a| a.as_array()) {
        return a.iter().any(|s| s.as_str() == Some("udp-hole-punch-v3"));
    }
    // 旧版 fan-files：直接就是 transfer::AppVersion 序列化（abilities 数组）
    if let Some(a) = v.get("abilities").and_then(|a| a.as_array()) {
        return a.iter().any(|s| s.as_str() == Some("udp-hole-punch-v3"));
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
    // 从对方候选列表里找 srflx（打洞目标 = 公网反射地址）
    let cands = match peer_msg {
        UdpMsg::Hello { candidates, .. } | UdpMsg::Ack { candidates, .. } => candidates,
        // Abort / FileMeta / ChunkStatus / RelayChunk：非打洞阶段消息，视为无打洞意图
        UdpMsg::Abort
        | UdpMsg::FileMeta { .. }
        | UdpMsg::ChunkStatus { .. }
        | UdpMsg::RelayChunk { .. } => return None,
    };
    let srflx = cands.iter().find(|c| c.kind == "srflx")?;
    let peer_addr: std::net::SocketAddr = srflx.addr.parse().ok()?;
    let nonce = match peer_msg {
        UdpMsg::Hello { nonce, .. } | UdpMsg::Ack { nonce, .. } => *nonce,
        UdpMsg::Abort | UdpMsg::FileMeta { .. } | UdpMsg::ChunkStatus { .. } | UdpMsg::RelayChunk { .. } => {
            return None
        }
    };
    // 对方 STUN 失败时会通告空候选 → 立即降级
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

/// 文件级 SHA-256（64KB 增量读，不整文件载入内存——大文件安全）
fn sha256_file(path: &std::path::Path) -> Result<String, String> {
    use sha2::Digest;
    let mut f = std::fs::File::open(path).map_err(|e| format!("打开 {}: {}", path.display(), e))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf)
            .map_err(|e| format!("读 {}: {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>())
}


/// 收尾：文件级 SHA-256 校验 partial → rename 到目标路径 → 清理清单。
/// 目标路径语义与 v1 get 一致：目录 → 目录/文件名；文件 → 该路径；None → 当前目录/文件名。
/// rename 优先，跨文件系统（EXDEV）降级为拷贝。返回最终文件路径。
fn finalize_partial(
    hash16: &str,
    file_name: &str,
    expected_sha256: &str,
    output: &Option<String>,
) -> Result<std::path::PathBuf, String> {
    let part = chunked::partial_path(hash16);
    // 文件级 SHA-256（增量读，不整文件载入内存）
    let digest = sha256_file(&part).map_err(|e| format!("校验 partial 失败: {e}"))?;
    if digest != expected_sha256 {
        return Err(format!(
            "文件级 SHA-256 校验失败: peer={expected_sha256} local={digest}"
        ));
    }
    let target_path: PathBuf = match output {
        Some(p) if PathBuf::from(p).is_dir() => PathBuf::from(p).join(file_name),
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(file_name),
    };
    if let Some(parent) = target_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::rename(&part, &target_path) {
        Ok(()) => {}
        Err(e) => {
            // 跨文件系统（EXDEV）：降级拷贝后删 partial
            eprintln!("  ⚠ rename 失败（{e}），降级拷贝");
            std::fs::copy(&part, &target_path)
                .map_err(|e| format!("拷贝到 {}: {}", target_path.display(), e))?;
            let _ = std::fs::remove_file(&part);
        }
    }
    chunked::clear_manifest(hash16);
    Ok(target_path)
}

// ---------- QUIC 多流并发分块传输（阶段 1，Task 2） ----------

/// QUIC 块头长度：u32 index(4) + u32 size(4) + [u8;32] sha256 = 40 字节（大端）。
/// 规格 §四：块级 SHA-256 携带在块头，接收方校验失败只重传该块。
pub const CHUNK_HEADER_LEN: usize = 4 + 4 + 32;

/// QUIC 多流并发传块（发送方）：信号量式 worker 池，`concurrency` 条并行流，
/// 每流传一块。流格式：40 字节块头（u32 index + u32 size + [u8;32] sha256，大端）
/// → 块数据 → 接收方校验后回 1 字节确认（0x6b = 通过，0x65 = 失败 → 重试）。
/// 每块独立 std::fs::File + seek(offset) 读块数据（避免共享游标并发问题）。
/// 失败重试 ≤3 次（attempts map），失败块放回队列尾由空闲 worker 再取；
/// 重试耗尽仍失败 → 返回 Err（已成功块由接收方清单保留，可续传）。
/// 进度回调：已传字节 / 总字节。返回已传字节总数。
async fn quic_send_chunks(
    conn: &quinn::Connection,
    path: &str,
    plan: &[chunked::Chunk],
    missing: &[u32],
    concurrency: usize,
    progress: impl FnMut(u64, u64) + Clone + Send + 'static,
) -> Result<u64, String> {
    use sha2::Digest;
    if !std::path::Path::new(path).exists() {
        return Err(format!("发送文件不存在: {path}"));
    }
    // 续传语义：进度总量 = 本次缺失块字节数（清单已完成块不计入）
    let total_bytes: u64 = missing.iter().map(|i| plan[*i as usize].size).sum();
    // 清单显示全部块已完成 → 无需传输（接收方自己会校验并收尾）
    if missing.is_empty() {
        return Ok(0);
    }
    let pending: std::sync::Arc<std::sync::Mutex<Vec<u32>>> =
        std::sync::Arc::new(std::sync::Mutex::new(missing.to_vec()));
    let attempts: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u32, u32>>> =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let failed: std::sync::Arc<std::sync::Mutex<Vec<u32>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let total_sent: std::sync::Arc<std::sync::atomic::AtomicU64> =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut progress = progress;
    progress(0, total_bytes);

    let n = concurrency.max(1);
    let mut handles = Vec::new();
    for _ in 0..n {
        let (pending, attempts, failed, total_sent) = (
            pending.clone(), attempts.clone(), failed.clone(), total_sent.clone(),
        );
        let conn = conn.clone();
        let plan = plan.to_vec();
        let path = path.to_string();
        let mut progress = progress.clone();
        handles.push(async_std::task::spawn(async move {
            loop {
                // 取下一个待传块（Mutex 队列，先到先得）
                let idx = {
                    let mut p = pending.lock().unwrap();
                    if p.is_empty() {
                        return; // 队列空 → 本 worker 结束
                    }
                    let i = p.remove(0);
                    i
                };
                let chunk = &plan[idx as usize];
                // 读块数据：独立 File + seek（避免共享游标并发问题）
                let mut f = match std::fs::File::open(&path) {
                    Ok(f) => f,
                    Err(e) => {
                        // 本地文件读失败重试无意义，直接判死
                        eprintln!("  ⚠ 打开文件失败: {e}");
                        failed.lock().unwrap().push(idx);
                        continue;
                    }
                };
                let mut buf = vec![0u8; chunk.size as usize];
                if std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(chunk.offset)).is_err() {
                    eprintln!("  ⚠ seek 块 {idx} 失败");
                    failed.lock().unwrap().push(idx);
                    continue;
                }
                if std::io::Read::read_exact(&mut f, &mut buf).is_err() {
                    eprintln!("  ⚠ 读块 {idx} 失败");
                    failed.lock().unwrap().push(idx);
                    continue;
                }
                // 块级 SHA-256（写进块头，接收方校验）
                let sha: [u8; 32] = sha2::Sha256::digest(&buf).into();
                // 重试循环 ≤3 次（open_bi / 发送 / 确认任一失败都换新流重试）
                let mut ok = false;
                for _ in 0..3 {
                    let (mut send, mut recv) = match conn.open_bi().await {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("  ⚠ open_bi: {e}");
                            continue;
                        }
                    };
                    // 块头：u32 index + u32 size + sha256（大端）
                    if send.write_all(&idx.to_be_bytes()).await.is_err() { continue; }
                    if send.write_all(&(chunk.size as u32).to_be_bytes()).await.is_err() { continue; }
                    if send.write_all(&sha).await.is_err() { continue; }
                    if send.write_all(&buf).await.is_err() { continue; }
                    if send.finish().is_err() { continue; }
                    // 等确认：0x6b = 校验通过；0x65 = 校验失败（重试）。
                    // 注意：read 返回的是读取字节数（Some(1)），须检查 ack[0] 内容。
                    let mut ack = [0u8; 1];
                    match recv.read(&mut ack).await {
                        Ok(Some(_)) if ack[0] == 0x6b => {
                            ok = true;
                            break;
                        }
                        Ok(Some(_)) if ack[0] == 0x65 => {
                            eprintln!("  ⚠ 块 {idx} 校验失败（0x65），重试");
                            continue;
                        }
                        _ => {
                            eprintln!("  ⚠ 块 {idx} 未收到确认，重试");
                            continue;
                        }
                    }
                }
                if ok {
                    total_sent.fetch_add(chunk.size, std::sync::atomic::Ordering::Relaxed);
                    progress(total_sent.load(std::sync::atomic::Ordering::Relaxed), total_bytes);
                } else {
                    // 失败：重试计数 +1，<3 次放回队列尾（下一轮由空闲 worker 再试）
                    let mut a = attempts.lock().unwrap();
                    let cnt = a.entry(idx).or_insert(0);
                    *cnt += 1;
                    if *cnt < 3 {
                        pending.lock().unwrap().push(idx);
                    } else {
                        failed.lock().unwrap().push(idx);
                    }
                }
            }
        }));
    }
    for h in handles {
        h.await;
    }
    // 有块重试耗尽仍失败 → 整体报错（已成功块保留在接收方清单，可续传）
    let failed = failed.lock().unwrap();
    if !failed.is_empty() {
        return Err(format!("以下块传输失败（重试耗尽）: {:?}", failed));
    }
    Ok(total_sent.load(std::sync::atomic::Ordering::Relaxed))
}

/// QUIC 多流并发收块（接收方）：accept_bi 循环 + 每流一个 task 并发处理。
/// 流格式与发送方对称：40 字节块头 → 块数据 → 回 1 字节确认（0x6b / 0x65）。
/// 块写入 partial 文件（chunked::partial_path，按 offset seek+write），每块完成
/// 更新清单（done 集合，原子持久化）。多流共享同一文件句柄，用 Mutex 串行化
/// seek+write——POSIX 下同一文件描述符的游标是共享的，不加锁并发 seek+write
/// 会互相抢占游标写错位置（不同 offset 的写入本身无冲突）。
/// `initial_done`：续传时清单里已完成的块（首次传输传空）。已完成块不计入
/// 本次进度/字节数（防二次计数）；重复收到已记账块（ack 丢失重发）→ 回 0x6b
/// 不重写。返回 (总完成块数, 本次收到字节数)。
async fn quic_recv_chunks(
    conn: &quinn::Connection,
    hash16: &str,
    file_name: &str,
    file_size: u64,
    chunk_size: u64,
    chunk_count: u32,
    initial_done: &[u32],
    progress: impl FnMut(u64, u64) + Clone + Send + 'static,
) -> Result<(u32, u64), String> {
    use sha2::Digest;
    let part_path = chunked::partial_path(hash16);
    if let Some(parent) = part_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 partial 目录: {e}"))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&part_path)
        .map_err(|e| format!("打开 partial 文件 {}: {}", part_path.display(), e))?;
    let file_arc: std::sync::Arc<std::sync::Mutex<std::fs::File>> =
        std::sync::Arc::new(std::sync::Mutex::new(file));
    // 初始 done = 清单已收块（续传断点；越界索引防御性过滤）。done 含初始集合，
    // 后续清单保存与完成度判断（done.len() >= chunk_count）自动涵盖续传语义。
    let plan = chunked::chunk_plan(file_size, chunk_size);
    let done: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<u32>>> =
        std::sync::Arc::new(std::sync::Mutex::new(
            initial_done.iter().copied().filter(|i| *i < chunk_count).collect(),
        ));
    // 初始已收字节（进度基线）与本次期望字节（= 缺失块总量）
    let initial_bytes: u64 = plan
        .iter()
        .filter(|c| done.lock().unwrap().contains(&c.index))
        .map(|c| c.size)
        .sum();
    let expected_bytes: u64 = file_size.saturating_sub(initial_bytes);
    let total_received: std::sync::Arc<std::sync::atomic::AtomicU64> =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut progress = progress;
    progress(initial_bytes, file_size);

    let mut handles = Vec::new();
    loop {
        // 块已收完 → 退出 accept 循环
        if done.lock().unwrap().len() as u32 >= chunk_count {
            break;
        }
        // accept_bi 与完成度轮询竞争：最后一块可能在 accept 阻塞期间完成，
        // 轮询保证收满后及时退出（否则会永远等一条不存在的流）
        let res = futures_lite::future::or(
            async { conn.accept_bi().await.map_err(|e| format!("accept_bi: {e}")) },
            async {
                async_io::Timer::after(Duration::from_millis(20)).await;
                Err(String::new())
            },
        )
        .await;
        let (send, recv) = match res {
            Ok(v) => v,
            Err(e) if e.is_empty() => continue, // 轮询：回到循环头检查完成度
            Err(e) => {
                // 连接关闭等真实错误：若块恰好已收完则正常结束，否则报错
                if done.lock().unwrap().len() as u32 >= chunk_count {
                    break;
                }
                return Err(e);
            }
        };
        let (file_arc, done, total_received) =
            (file_arc.clone(), done.clone(), total_received.clone());
        let hash16 = hash16.to_string();
        let file_name = file_name.to_string();
        let mut progress = progress.clone();
        handles.push(async_std::task::spawn(async move {
            let mut recv = recv;
            let mut send = send;
            // 读块头：u32 index + u32 size + [u8;32] sha256（大端）
            let mut hdr = [0u8; CHUNK_HEADER_LEN];
            if recv.read_exact(&mut hdr).await.is_err() {
                return; // 断流：不确认，发送方自会重试
            }
            let idx = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
            let size = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
            // 限长 64MB（L3 教训：防恶意/损坏 peer 声明超大块长导致 OOM）
            if size as u64 > 64 * 1024 * 1024 || idx as usize >= chunk_count as usize {
                let _ = send.write_all(&[0x65]).await; // 协议异常 → nack
                return;
            }
            // 重复块（ack 丢失重发 / 续传边界）：已记账的块直接回 0x6b 不重写、
            // 不计数（防二次计数——total_received 与清单 done 均为幂等集合）
            if done.lock().unwrap().contains(&idx) {
                let _ = send.write_all(&[0x6b]).await;
                return;
            }
            let mut buf = vec![0u8; size as usize];
            if recv.read_exact(&mut buf).await.is_err() {
                return; // 断流：不确认，发送方自会重试
            }
            // 块级 SHA-256 校验：不匹配 → nack（0x65），发送方只重传该块
            let digest: [u8; 32] = sha2::Sha256::digest(&buf).into();
            if digest != hdr[8..CHUNK_HEADER_LEN] {
                let _ = send.write_all(&[0x65]).await;
                return;
            }
            // 按 offset 写入 partial 文件（offset = index × chunk_size，块连续排列；
            // 共享句柄 + 写锁：POSIX 文件游标共享，seek+write 必须原子成对）
            {
                let mut f = file_arc.lock().unwrap();
                let offset = idx as u64 * chunk_size;
                if std::io::Seek::seek(&mut *f, std::io::SeekFrom::Start(offset)).is_err()
                    || std::io::Write::write_all(&mut *f, &buf).is_err()
                {
                    return; // 写入失败：不确认（发送方重试）
                }
            }
            // 确认（0x6b = 校验通过）
            let _ = send.write_all(&[0x6b]).await;
            // 更新清单：done 集合 + 原子持久化（锁内完成，防并发清单丢失更新）
            let mut d = done.lock().unwrap();
            d.insert(idx);
            total_received.fetch_add(size as u64, std::sync::atomic::Ordering::Relaxed);
            let m = chunked::Manifest {
                file_name,
                file_size,
                chunk_size,
                done: d.iter().copied().collect(),
            };
            let _ = chunked::save_manifest(&hash16, &m);
            progress(
                initial_bytes + total_received.load(std::sync::atomic::Ordering::Relaxed),
                file_size,
            );
        }));
    }
    for h in handles {
        h.await;
    }
    let chunks = done.lock().unwrap().len() as u32;
    let bytes = total_received.load(std::sync::atomic::Ordering::Relaxed);
    // 续传语义：完成度看总块数（含清单初始块），字节数看本次缺失块
    if chunks != chunk_count {
        return Err(format!(
            "仅收到 {chunks}/{chunk_count} 块（可能断线，清单已保留可续传）"
        ));
    }
    if bytes != expected_bytes {
        return Err(format!(
            "本次收到 {bytes} 字节 ≠ 期望 {expected_bytes}（{chunks}/{chunk_count} 块，可能断线）"
        ));
    }
    Ok((chunks, bytes))
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

/// QUIC 建连后的接收方流程（host / srflx 两路径共用）：
/// 收 FileMeta（wormhole）→ 查清单 → 回 ChunkStatus → 收缺失块写 partial →
/// 文件级 SHA-256 校验 → rename 到目标路径 → 清理清单。返回 (文件名, 本次收到字节)。
async fn quic_recv_resume(
    conn: &quinn::Connection,
    wormhole: &mut Wormhole,
    output: &Option<String>,
    progress: impl FnMut(u64, u64) + Clone + Send + 'static,
) -> Result<(String, u64), String> {
    // ① 收 FileMeta（发送方在 QUIC 建连后即发）
    let (name, size, sha256, chunk_size, chunk_count) =
        match recv_udp_msg(wormhole, Duration::from_secs(15), "file-meta").await? {
            Some(UdpMsg::FileMeta { name, size, sha256, chunk_size, chunk_count }) => {
                (name, size, sha256, chunk_size, chunk_count)
            }
            Some(_) => return Err("收到意外的 UDP 握手消息".into()),
            None => return Err("file-meta 超时".into()),
        };
    // 清单自洽校验：chunk_count 必须等于本地分块计划块数（防恶意/损坏的 FileMeta
    // 声称少于实际块数——块数不足会让 accept 循环提前结束，partial 缺尾部数据，
    // 文件级校验虽能兜底，但应尽早报错而非走完整个传输）。chunk_plan 自适应放大
    // 块大小（>512 块），发送方发的 chunk_size 已是放大后的值，双方结果一致。
    let expect_chunks = chunked::chunk_plan(size, chunk_size).len() as u32;
    if chunk_count != expect_chunks {
        return Err(format!(
            "FileMeta chunk_count={chunk_count} 与分块计划不一致（应为 {expect_chunks}）"
        ));
    }
    // ② 清单比对：hash16 = 文件级 SHA-256 前 16 字符；清单匹配（size/chunk_size
    //    一致）→ 已收块集合；不匹配 → 空（全量传输）
    let hash16: String = sha256.chars().take(16).collect();
    let done = match chunked::load_manifest(&hash16) {
        Some(m) if m.file_size == size && m.chunk_size == chunk_size => m.done,
        _ => Vec::new(),
    };
    if !done.is_empty() {
        eprintln!("  ♻ 清单命中 {} 块，续传缺失块（总 {} 块）", done.len(), chunk_count);
    }
    // ③ 回 ChunkStatus（清单回执；空 = 全量传输）
    wormhole
        .send_json(&UdpMsg::ChunkStatus { done: done.clone() })
        .await
        .map_err(|e| format!("send chunk-status: {e}"))?;
    // ④ 多流收缺失块（写 partial + 逐块更新清单）
    let (chunks, bytes) = quic_recv_chunks(
        conn, &hash16, &name, size, chunk_size, chunk_count, &done, progress,
    )
    .await?;
    eprintln!("  ✅ 收满 {chunks} 块（本次 {bytes} 字节），文件级校验…");
    // ⑤ 文件级 SHA-256 校验 → rename → 清理清单
    let target = match finalize_partial(&hash16, &name, &sha256, output) {
        Ok(t) => t,
        Err(e) => {
            // 文件级校验失败 = partial 与清单不一致（数据可疑），清理清单让重试
            // 走全量传输（自愈），避免清单死锁（永远缺同样的块）
            eprintln!("  ⚠ 收尾失败，清理清单以便下次全量重传");
            chunked::clear_manifest(&hash16);
            return Err(e);
        }
    };
    eprintln!("  ✅ 文件级 SHA-256 校验通过，已保存: {}", target.display());
    Ok((name, bytes))
}

// ---------- relay 多线程分块传输（阶段 2，Task 5） ----------
//
// 每块一个**独立** magic-wormhole 会话（MailboxConnection::create 生成块配对码），
// 经主通道 RelayChunk {code, index} 协调：发送方 worker 池（concurrency 个，
// 每 worker 一个会话）并发传块；接收方按码发起 transfer get 收块（OffsetWriter
// 写 partial 对应 offset，逐块原子更新清单）。块级 SHA-256 由 magic-wormhole
// v1 协议自带校验；失败重试 = 新会话 + 新码（发送方驱动，接收方只消费主通道码）。

/// relay 分块并发数（每 worker 一个独立 wormhole 会话；与 QUIC 并发一致）
const RELAY_CHUNK_CONCURRENCY: usize = 4;

/// relay 块 worker → 主任务的控制消息（std mpsc，FIFO 保序）
enum RelayCodeMsg {
    /// 新块配对码（index + 独立会话 code），需经主通道转发 RelayChunk
    Relay { index: u32, code: String },
    /// worker 结束（必在其最后一条 Relay 之后入队 → 收满 Done 即所有码已转发）
    Done,
}

/// 块级 offset 写入包装：把一次 accept 的完整块数据流写入 partial 的对应 offset。
/// magic-wormhole accept 对内容处理器是整文件顺序写（多次 poll_write 记录）。
/// **每次写入前必须 seek**：多块并发共享同一 partial 句柄，文件游标是共享的
/// 单一位置——别的 worker seek+write 会移动游标，若只 seek 一次，本 worker 后续
/// 记录会写到游标当前处（错位）。seek + write 持锁原子成对完成（与 quic_recv_chunks
/// 同一约束）。实现 futures_lite::io::AsyncWrite（= futures_io::AsyncWrite，
/// accept 的 W 约束）。
struct OffsetWriter {
    file: std::sync::Arc<std::sync::Mutex<std::fs::File>>,
    offset: u64,
    /// 本 writer 已写入字节数（Cell：持锁期间仍可更新，见 poll_write）
    written: std::cell::Cell<u64>,
}

impl futures_lite::io::AsyncWrite for OffsetWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let mut f = self.file.lock().unwrap();
        // 每次写入前 seek 到 offset + 已写字节（不能只 seek 一次：共享游标会被
        // 其它 worker 移动，见结构体注释）
        if let Err(e) = std::io::Seek::seek(
            &mut *f,
            std::io::SeekFrom::Start(self.offset + self.written.get()),
        ) {
            return std::task::Poll::Ready(Err(e));
        }
        match std::io::Write::write(&mut *f, buf) {
            Ok(n) => {
                self.written.set(self.written.get() + n as u64);
                std::task::Poll::Ready(Ok(n))
            }
            Err(e) => std::task::Poll::Ready(Err(e)),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let mut f = self.file.lock().unwrap();
        std::task::Poll::Ready(std::io::Write::flush(&mut *f))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        // 数据已直写文件，close 只需 flush（accept 收尾会调用 close）
        self.poll_flush(cx)
    }
}

/// 单块 relay 发送：新建**独立** magic-wormhole 会话（生成块配对码）→ 配对码经
/// code_tx 交主任务转发 RelayChunk → transfer::send_file 传块数据。
/// 块数据为内存缓冲（4MB 级）——send_file 接受任意 AsyncRead，无需临时文件；
/// 块级 SHA-256 由 magic-wormhole v1 协议自带（offer 内 digest 校验）。
async fn relay_send_one_chunk(
    code_tx: &std::sync::mpsc::Sender<RelayCodeMsg>,
    hints: &[transit::RelayHint],
    buf: &[u8],
    index: u32,
) -> Result<(), String> {
    let mailbox = MailboxConnection::create(app_config(), 3)
        .await
        .map_err(|e| format!("创建 relay 会话: {e}"))?;
    let code = mailbox.code().to_string();
    code_tx
        .send(RelayCodeMsg::Relay { index, code })
        .map_err(|_| "主通道已关闭".to_string())?;
    let wormhole = Wormhole::connect(mailbox)
        .await
        .map_err(|e| format!("Wormhole 连接: {e}"))?;
    let mut cursor = async_std::io::Cursor::new(buf.to_vec());
    transfer::send_file(
        wormhole,
        hints.to_vec(),
        &mut cursor,
        format!("chunk-{index}"),
        buf.len() as u64,
        transit::Abilities::ALL,
        |_| {},       // relay 路径连接类型不展示
        |_, _| {},    // 块粒度进度由 relay_send_chunks 聚合（与 quic_send_chunks 一致）
        std::future::pending::<()>(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// relay 分块发送：每个缺失块一个独立 wormhole 会话（配对码经主通道 RelayChunk
/// 告知接收方），`concurrency` 个 worker 并行。worker 取队列块 → 读块数据 →
/// 建会话 → 交主任务转发配对码 → send_file 传块；失败重试 ≤3 次（新会话 + 新码）。
/// 主通道消息流：N × RelayChunk（发送方顺序，与接收方 get 一一对应）。
/// 返回已传字节总数（= 缺失块字节和）。
async fn relay_send_chunks(
    main_wormhole: &mut Wormhole,
    path: &str,
    plan: &[chunked::Chunk],
    missing: &[u32],
    concurrency: usize,
    progress: impl FnMut(u64, u64) + Clone + Send + 'static,
) -> Result<u64, String> {
    if !std::path::Path::new(path).exists() {
        return Err(format!("发送文件不存在: {path}"));
    }
    // 续传语义：进度总量 = 本次缺失块字节数（清单已完成块不计入）
    let total_bytes: u64 = missing.iter().map(|i| plan[*i as usize].size).sum();
    // 清单显示全部块已完成 → 无需传输（接收方自己会校验并收尾）
    if missing.is_empty() {
        return Ok(0);
    }
    let pending: std::sync::Arc<std::sync::Mutex<Vec<u32>>> =
        std::sync::Arc::new(std::sync::Mutex::new(missing.to_vec()));
    let attempts: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u32, u32>>> =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let failed: std::sync::Arc<std::sync::Mutex<Vec<u32>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let total_sent: std::sync::Arc<std::sync::atomic::AtomicU64> =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    // (index, code) 转发通道：worker 生成配对码 → 主任务经主通道发 RelayChunk
    let (code_tx, code_rx) = std::sync::mpsc::channel::<RelayCodeMsg>();
    let hints = relay_hints();
    let mut progress = progress;
    progress(0, total_bytes);

    let n = concurrency.max(1);
    let mut handles = Vec::new();
    for _ in 0..n {
        let (pending, attempts, failed, total_sent) = (
            pending.clone(), attempts.clone(), failed.clone(), total_sent.clone(),
        );
        let code_tx = code_tx.clone();
        let hints = hints.clone();
        let plan = plan.to_vec();
        let path = path.to_string();
        let mut progress = progress.clone();
        handles.push(async_std::task::spawn(async move {
            loop {
                // 取下一个待传块（Mutex 队列，先到先得；空 → 本 worker 结束）
                let idx = {
                    let mut p = pending.lock().unwrap();
                    if p.is_empty() {
                        break;
                    }
                    p.remove(0)
                };
                let chunk = &plan[idx as usize];
                // 读块数据：独立 File + seek（避免共享游标并发问题）；读入内存后
                // 重试循环复用同一缓冲，无需反复读盘
                let mut f = match std::fs::File::open(&path) {
                    Ok(f) => f,
                    Err(e) => {
                        // 本地文件读失败重试无意义，直接判死
                        eprintln!("  ⚠ 打开文件失败: {e}");
                        failed.lock().unwrap().push(idx);
                        continue;
                    }
                };
                let mut buf = vec![0u8; chunk.size as usize];
                if std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(chunk.offset)).is_err()
                    || std::io::Read::read_exact(&mut f, &mut buf).is_err()
                {
                    eprintln!("  ⚠ 读块 {idx} 失败");
                    failed.lock().unwrap().push(idx);
                    continue;
                }
                // 重试 ≤3：每轮新建独立会话 + 新配对码（接收方经主通道拿新码重 get）
                let mut ok = false;
                for attempt in 0..3 {
                    match relay_send_one_chunk(&code_tx, &hints, &buf, idx).await {
                        Ok(()) => {
                            ok = true;
                            break;
                        }
                        Err(e) => {
                            eprintln!("  ⚠ 块 {idx} relay 会话失败（第 {} 次）: {e}", attempt + 1);
                        }
                    }
                }
                if ok {
                    total_sent.fetch_add(chunk.size, std::sync::atomic::Ordering::Relaxed);
                    progress(total_sent.load(std::sync::atomic::Ordering::Relaxed), total_bytes);
                } else {
                    // 失败：重试计数 +1，<3 次放回队列尾（下一轮由空闲 worker 再试）
                    let mut a = attempts.lock().unwrap();
                    let cnt = a.entry(idx).or_insert(0);
                    *cnt += 1;
                    if *cnt < 3 {
                        pending.lock().unwrap().push(idx);
                    } else {
                        failed.lock().unwrap().push(idx);
                    }
                }
            }
            // Done 必在最后一条 Relay 之后入队（mpsc FIFO 保证主任务转发完整性）
            let _ = code_tx.send(RelayCodeMsg::Done);
        }));
    }
    // 主任务：边转发 RelayChunk 码边等 worker 收尾（std mpsc try_recv 轮询，
    // 不阻塞 executor）。转发失败 = 主通道断裂 → 整体报错（worker 随进程退出）。
    let mut remaining = n;
    while remaining > 0 {
        match code_rx.try_recv() {
            Ok(RelayCodeMsg::Relay { index, code }) => {
                main_wormhole
                    .send_json(&UdpMsg::RelayChunk { code, index })
                    .await
                    .map_err(|e| format!("send relay-chunk: {e}"))?;
            }
            Ok(RelayCodeMsg::Done) => remaining -= 1,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                async_io::Timer::after(Duration::from_millis(5)).await;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        }
    }
    for h in handles {
        h.await;
    }
    // 有块重试耗尽仍失败 → 整体报错（已成功块保留在接收方清单，可续传）
    let failed = failed.lock().unwrap();
    if !failed.is_empty() {
        return Err(format!("以下块 relay 传输失败（重试耗尽）: {:?}", failed));
    }
    Ok(total_sent.load(std::sync::atomic::Ordering::Relaxed))
}

/// 单块 relay 接收：用块配对码连接独立会话 → request_file + accept 收块 →
/// OffsetWriter 写 partial 对应 offset → 更新清单（done 集合 + 原子持久化，
/// 锁内完成防并发丢失更新）。重复块（发送方重试 / ack 丢失重发）幂等：不重复
/// 计数、不重复记账。失败 → 返回 Err，由发送方换新配对码重试。
async fn relay_recv_one_chunk<P>(
    index: u32,
    code_str: &str,
    expect_size: u64,
    offset: u64,
    hints: &[transit::RelayHint],
    file_arc: std::sync::Arc<std::sync::Mutex<std::fs::File>>,
    hash16: &str,
    file_name: &str,
    file_size: u64,
    chunk_size: u64,
    initial_bytes: u64,
    done: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<u32>>>,
    total_received: std::sync::Arc<std::sync::atomic::AtomicU64>,
    progress: std::sync::Arc<std::sync::Mutex<P>>,
) -> Result<(), String>
where
    P: FnMut(u64, u64) + Send + 'static,
{
    let code: Code = code_str
        .parse()
        .map_err(|e| format!("无效配对码 '{code_str}': {e}"))?;
    let mailbox = MailboxConnection::connect(app_config(), code, false)
        .await
        .map_err(|e| format!("连接块会话: {e}"))?;
    let wormhole = Wormhole::connect(mailbox)
        .await
        .map_err(|e| format!("Wormhole 连接: {e}"))?;
    let req = match transfer::request_file(
        wormhole,
        hints.to_vec(),
        transit::Abilities::ALL,
        std::future::pending::<()>(),
    )
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return Err("对方取消传输".into()),
        Err(e) => return Err(format!("请求块 {index}: {e}")),
    };
    // 防御：offer 声明大小必须与块计划一致（损坏/错配会话 → 直接失败让发送方重试）
    if req.file_size() != expect_size {
        return Err(format!(
            "块 {index} offer 大小 {} ≠ 期望 {expect_size}",
            req.file_size()
        ));
    }
    let mut writer = OffsetWriter {
        file: file_arc,
        offset,
        written: std::cell::Cell::new(0),
    };
    req.accept(
        &|_| {},    // 连接类型不展示（relay 路径）
        |_, _| {},  // 块粒度进度由 relay_recv_chunks 聚合
        &mut writer,
        std::future::pending::<()>(),
    )
    .await
    .map_err(|e| format!("收块 {index}: {e}"))?;
    // 更新清单（原子）；重复块幂等——已记账的块不重写 done、不计数
    let mut d = done.lock().unwrap();
    if d.insert(index) {
        total_received.fetch_add(expect_size, std::sync::atomic::Ordering::Relaxed);
        let m = chunked::Manifest {
            file_name: file_name.to_string(),
            file_size,
            chunk_size,
            done: d.iter().copied().collect(),
        };
        let _ = chunked::save_manifest(hash16, &m);
        let mut p = progress.lock().unwrap();
        p(
            initial_bytes + total_received.load(std::sync::atomic::Ordering::Relaxed),
            file_size,
        );
    }
    Ok(())
}

/// relay 分块接收：主任务收 RelayChunk（块配对码）→ 分发给 worker 池（并发上限
/// `concurrency`），每个 worker 用配对码发起独立 transfer get 收块写 partial。
/// 结束条件：done 满（全部块）→ 等 in-flight worker 收尾后校验返回；
/// 收到 Abort（发送方放弃）→ 报错（清单保留可续传）；主通道 600s 超时 → 报错。
/// 返回 (总完成块数, 本次收到字节数)。
async fn relay_recv_chunks(
    main_wormhole: &mut Wormhole,
    hash16: &str,
    file_name: &str,
    plan: &[chunked::Chunk],
    concurrency: usize,
    progress: impl FnMut(u64, u64) + Clone + Send + 'static,
) -> Result<(u32, u64), String> {
    let file_size: u64 = plan.last().map(|c| c.offset + c.size).unwrap_or(0);
    let chunk_size: u64 = plan.first().map(|c| c.size).unwrap_or(0);
    let part_path = chunked::partial_path(hash16);
    if let Some(parent) = part_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 partial 目录: {e}"))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&part_path)
        .map_err(|e| format!("打开 partial 文件 {}: {}", part_path.display(), e))?;
    let file_arc: std::sync::Arc<std::sync::Mutex<std::fs::File>> =
        std::sync::Arc::new(std::sync::Mutex::new(file));
    // done = 清单已收块（续传断点；越界索引防御性过滤）
    let done: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<u32>>> =
        std::sync::Arc::new(std::sync::Mutex::new(
            chunked::load_manifest(hash16)
                .filter(|m| m.file_size == file_size && m.chunk_size == chunk_size)
                .map(|m| m.done.into_iter().filter(|i| *i < plan.len() as u32).collect())
                .unwrap_or_default(),
        ));
    // 初始已收字节（进度基线）与本次期望字节（= 缺失块总量）
    let initial_bytes: u64 = plan
        .iter()
        .filter(|c| done.lock().unwrap().contains(&c.index))
        .map(|c| c.size)
        .sum();
    let expected_bytes: u64 = file_size.saturating_sub(initial_bytes);
    let total_received: std::sync::Arc<std::sync::atomic::AtomicU64> =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    // worker 活跃数（并发上限）；进度回调经 Arc<Mutex> 供 worker 共享
    let active: std::sync::Arc<std::sync::atomic::AtomicUsize> =
        std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hints = relay_hints();
    let mut progress = progress;
    progress(initial_bytes, file_size);
    let progress: std::sync::Arc<std::sync::Mutex<_>> =
        std::sync::Arc::new(std::sync::Mutex::new(progress));

    let n = concurrency.max(1);
    let mut handles = Vec::new();
    // 主通道整体超时：发送方放弃（未发 Abort）/ 崩溃时兜底，清单保留可续传
    let deadline = std::time::Instant::now() + Duration::from_secs(600);
    loop {
        // 收满 → 退出主循环（最后一块可能在 accept 阻塞期间完成，轮询保证及时退出）
        if done.lock().unwrap().len() >= plan.len() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            for h in handles {
                h.await;
            }
            return Err("等待 relay 块超时（清单已保留，可续传）".into());
        }
        // 收 RelayChunk（20ms 轮询，与 quic_recv_chunks 的 accept 轮询同模式）
        let res = futures_lite::future::or(
            async {
                match main_wormhole.receive_json::<UdpMsg>().await {
                    Ok(Ok(m)) => Ok(Some(m)),
                    Ok(Err(e)) => Err(format!("relay-chunk 解析失败: {e}")),
                    Err(e) => Err(format!("relay-chunk 接收失败: {e}")),
                }
            },
            async {
                async_io::Timer::after(Duration::from_millis(20)).await;
                Ok(None)
            },
        )
        .await;
        match res {
            Ok(Some(UdpMsg::RelayChunk { code, index })) => {
                if index as usize >= plan.len() {
                    return Err(format!("收到越界块 index {index}（块数 {}）", plan.len()));
                }
                // 并发限制：等空闲槽（发送方 worker 在等我们 get，槽位释放即推进，
                // 不会死锁）；期间 done 可能收满 → 退出不再分发
                while active.load(std::sync::atomic::Ordering::Acquire) >= n {
                    if done.lock().unwrap().len() >= plan.len() {
                        break;
                    }
                    async_io::Timer::after(Duration::from_millis(5)).await;
                }
                if done.lock().unwrap().len() >= plan.len() {
                    break;
                }
                let chunk = &plan[index as usize];
                active.fetch_add(1, std::sync::atomic::Ordering::Release);
                let (file_arc, done, total_received, active) = (
                    file_arc.clone(), done.clone(), total_received.clone(), active.clone(),
                );
                let hash16 = hash16.to_string();
                let file_name = file_name.to_string();
                let progress = progress.clone();
                let hints = hints.clone();
                let (code2, chunk_size2, chunk_offset) = (code, chunk.size, chunk.offset);
                handles.push(async_std::task::spawn(async move {
                    let r = relay_recv_one_chunk(
                        index,
                        &code2,
                        chunk_size2,
                        chunk_offset,
                        &hints,
                        file_arc,
                        &hash16,
                        &file_name,
                        file_size,
                        chunk_size,
                        initial_bytes,
                        done,
                        total_received,
                        progress,
                    )
                    .await;
                    if let Err(e) = &r {
                        // 会话失败：发送方会换新配对码重试，这里只记录不中断
                        eprintln!("  ⚠ 块 {index} relay 接收失败（等待发送方重试）: {e}");
                    }
                    active.fetch_sub(1, std::sync::atomic::Ordering::Release);
                }));
            }
            Ok(Some(UdpMsg::Abort)) => {
                for h in handles {
                    h.await;
                }
                return Err("对方中止 relay 传输（清单已保留，可续传）".into());
            }
            Ok(Some(_)) => {
                for h in handles {
                    h.await;
                }
                return Err("收到意外的 UDP 握手消息".into());
            }
            Ok(None) => { /* 轮询：回到循环头检查完成度/超时 */ }
            Err(e) => {
                for h in handles {
                    h.await;
                }
                return Err(e);
            }
        }
    }
    // 收满：等所有 in-flight worker 完成（最后一块的清单更新可能仍在写）
    for h in handles {
        h.await;
    }
    let chunks = done.lock().unwrap().len() as u32;
    let bytes = total_received.load(std::sync::atomic::Ordering::Relaxed);
    // 续传语义：完成度看总块数（含清单初始块），字节数看本次缺失块
    if chunks as usize != plan.len() {
        return Err(format!(
            "仅收到 {chunks}/{} 块（可能断线，清单已保留可续传）",
            plan.len()
        ));
    }
    if bytes != expected_bytes {
        return Err(format!(
            "本次收到 {bytes} 字节 ≠ 期望 {expected_bytes}（{chunks}/{} 块，可能断线）",
            plan.len()
        ));
    }
    Ok((chunks, bytes))
}

/// relay 分块发送主流程（v3，UDP 直连失败后的兜底；FAN_NO_UDP=1 直接走这里）：
/// 后台算文件级 SHA-256 → 发 FileMeta → 收 ChunkStatus（清单回执）→
/// relay_send_chunks 并发传缺失块。partial + 清单与 QUIC 路径直接复用（只传
/// missing）。返回已传字节总数。
async fn relay_send_resume(
    wormhole: &mut Wormhole,
    send_target: &str,
    display_name: &str,
    progress: impl FnMut(u64, u64) + Clone + Send + 'static,
) -> Result<u64, String> {
    let filesize = std::fs::metadata(send_target)
        .map_err(|e| format!("文件元数据: {e}"))?
        .len();
    let plan = chunked::chunk_plan(filesize, chunked::DEFAULT_CHUNK_SIZE);
    let chunk_size = plan.first().map(|c| c.size).unwrap_or(0);
    // 文件级 SHA-256：后台线程计算（relay 相位无时间窗口，仅避免阻塞主协程太久）。
    // 接收方等 FileMeta 的超时窗口为 300s（> 大文件哈希时间），不会误判降级。
    eprintln!("  🔎 计算文件级 SHA-256…");
    let (hash_tx, hash_rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let hash_path = send_target.to_string();
    std::thread::spawn(move || {
        let _ = hash_tx.send(sha256_file(std::path::Path::new(&hash_path)));
    });
    let sha256 = match hash_rx.recv() {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(format!("文件级 SHA-256 计算失败: {e}")),
        Err(_) => return Err("哈希线程异常退出".into()),
    };
    wormhole
        .send_json(&UdpMsg::FileMeta {
            name: display_name.to_string(),
            size: filesize,
            sha256,
            chunk_size,
            chunk_count: plan.len() as u32,
        })
        .await
        .map_err(|e| format!("send file-meta: {e}"))?;
    // 收清单回执 → 缺失块 = 全量块 − 已收块（空回执 = 全量传输）
    let done = match recv_udp_msg(wormhole, Duration::from_secs(15), "chunk-status").await? {
        Some(UdpMsg::ChunkStatus { done }) => done,
        Some(_) => return Err("收到意外的 UDP 握手消息".into()),
        None => return Err("chunk-status 超时".into()),
    };
    let done_set: std::collections::BTreeSet<u32> = done.into_iter().collect();
    let missing = chunked::missing_chunks(plan.len(), &done_set);
    if !done_set.is_empty() {
        eprintln!("  ♻ 清单回执 {} 块已完成，续传缺失 {} 块", done_set.len(), missing.len());
    }
    eprintln!("  📡 relay 分块：{} 块经独立 wormhole 会话并发传输…", missing.len());
    relay_send_chunks(
        wormhole,
        send_target,
        &plan,
        &missing,
        RELAY_CHUNK_CONCURRENCY,
        progress,
    )
    .await
}

/// relay 分块接收主流程（v3，UDP 失败降级或 FAN_NO_UDP 直连 relay）：
/// 收 FileMeta（若 hello 阶段已消费则直接复用）→ 查清单 → 回 ChunkStatus →
/// relay_recv_chunks 收缺失块写 partial → 文件级 SHA-256 校验 → rename →
/// 清理清单。与 quic_recv_resume 完全对称（partial + 清单跨路径复用）。
/// 返回 (文件名, 本次收到字节)。
async fn relay_recv_resume(
    wormhole: &mut Wormhole,
    output: &Option<String>,
    initial_meta: Option<UdpMsg>,
    progress: impl FnMut(u64, u64) + Clone + Send + 'static,
) -> Result<(String, u64), String> {
    // ① 收 FileMeta（hello 阶段已收到则复用，不重收）。超时窗口 300s：
    //    发送方在后台算大文件哈希后才发 FileMeta，不能沿用 15s 的 udp-hello
    //    窗口（v1 transit 消息仍会被解析失败快速识别 → 走 v1 路径）。
    let (name, size, sha256, chunk_size, chunk_count) = match initial_meta {
        Some(UdpMsg::FileMeta { name, size, sha256, chunk_size, chunk_count }) => {
            (name, size, sha256, chunk_size, chunk_count)
        }
        Some(_) => return Err("收到意外的 UDP 握手消息".into()),
        None => match recv_udp_msg(wormhole, Duration::from_secs(300), "file-meta").await? {
            Some(UdpMsg::FileMeta { name, size, sha256, chunk_size, chunk_count }) => {
                (name, size, sha256, chunk_size, chunk_count)
            }
            Some(_) => return Err("收到意外的 UDP 握手消息".into()),
            None => return Err("file-meta 超时".into()),
        },
    };
    // 清单自洽校验：chunk_count 必须等于本地分块计划块数（防恶意/损坏的 FileMeta，
    // 同 quic_recv_resume 的 Task 4 C1 校验）
    let expect_chunks = chunked::chunk_plan(size, chunk_size).len() as u32;
    if chunk_count != expect_chunks {
        return Err(format!(
            "FileMeta chunk_count={chunk_count} 与分块计划不一致（应为 {expect_chunks}）"
        ));
    }
    // ② 清单比对：hash16 = 文件级 SHA-256 前 16 字符；清单匹配 → 已收块集合
    let hash16: String = sha256.chars().take(16).collect();
    let done = match chunked::load_manifest(&hash16) {
        Some(m) if m.file_size == size && m.chunk_size == chunk_size => m.done,
        _ => Vec::new(),
    };
    if !done.is_empty() {
        eprintln!("  ♻ 清单命中 {} 块，续传缺失块（总 {} 块）", done.len(), chunk_count);
    }
    // ③ 回 ChunkStatus（清单回执；空 = 全量传输）
    wormhole
        .send_json(&UdpMsg::ChunkStatus { done: done.clone() })
        .await
        .map_err(|e| format!("send chunk-status: {e}"))?;
    // ④ 多会话并发收缺失块（写 partial + 逐块更新清单）
    let plan = chunked::chunk_plan(size, chunk_size);
    eprintln!("  📡 relay 分块：{} 块经独立 wormhole 会话并发传输…", chunk_count);
    let (chunks, bytes) = relay_recv_chunks(
        wormhole,
        &hash16,
        &name,
        &plan,
        RELAY_CHUNK_CONCURRENCY,
        progress,
    )
    .await?;
    eprintln!("  ✅ 收满 {chunks} 块（本次 {bytes} 字节），文件级校验…");
    // ⑤ 文件级 SHA-256 校验 → rename → 清理清单
    let target = match finalize_partial(&hash16, &name, &sha256, output) {
        Ok(t) => t,
        Err(e) => {
            // 文件级校验失败 = partial 与清单不一致（数据可疑），清理清单让重试
            // 走全量传输（自愈），避免清单死锁（永远缺同样的块）
            eprintln!("  ⚠ 收尾失败，清理清单以便下次全量重传");
            chunked::clear_manifest(&hash16);
            return Err(e);
        }
    };
    eprintln!("  ✅ 文件级 SHA-256 校验通过，已保存: {}", target.display());
    Ok((name, bytes))
}

/// 接收方 relay 分块入口（get 用）：带进度显示的 relay_recv_resume 封装。
/// 成功 → Ok((文件名, 本次字节))；失败 → Err（已含错误说明）。
async fn relay_get_result(
    wormhole: &mut Wormhole,
    output: &Option<String>,
    initial_meta: Option<UdpMsg>,
) -> Result<(String, u64), String> {
    relay_recv_resume(wormhole, output, initial_meta, |d, t| {
        if t > 0 {
            eprintln!("\r  进度: {}/{} ({:.0}%)", d, t, d as f64 / t as f64 * 100.0);
        }
    })
    .await
}

/// UDP 直连发送（发送方 = QUIC 服务端，ICE-lite 多候选监听）：
/// 收集 host + srflx 候选 → hello（完整候选）→ ack → 多端 quic_listen → accept 循环
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
    // 预备：文件大小 + 分块计划（纯内存计算，快）
    let filesize = std::fs::metadata(send_target)
        .map_err(|e| format!("文件元数据: {e}"))?
        .len();
    let plan = chunked::chunk_plan(filesize, chunked::DEFAULT_CHUNK_SIZE);
    let chunk_size = plan.first().map(|c| c.size).unwrap_or(0);
    // 文件级 SHA-256 放入后台线程（64KB 增量读）——与候选收集/hello/打洞/accept
    // 并行。大文件（>20-30GB）哈希可能超过 13s，不能阻塞在 hello 之前：否则接收方
    // 15s 等 udp-hello 超时降级 v1，迟到的 hello 污染 v1 消息流 → 硬失败。
    // 哈希失败（文件读错）→ 通道收 Err → 本端发 Abort 降级 relay。
    eprintln!("  🔎 后台计算文件级 SHA-256（与打洞并行，不阻塞 hello）…");
    let (hash_tx, hash_rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let hash_path = send_target.to_string();
    std::thread::spawn(move || {
        let _ = hash_tx.send(sha256_file(std::path::Path::new(&hash_path)));
    });

    // ① 收集候选：
    //    - host：每个本地 IPv4 接口绑一个 socket（ip:0 随机端口），同网段对端免打洞直连
    //    - srflx：0.0.0.0:0 + UDP STUN（同 socket 保证端口映射一致）
    //    注意：STUN 失败也要发 hello（host 候选仍可直连）——接收方在等 hello，
    //    不发会让它把 v1 transit 消费掉导致协议错乱。
    let mut host_socks: Vec<std::net::UdpSocket> = Vec::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    for ip in local_ipv4_addrs() {
        if let Ok(s) = std::net::UdpSocket::bind((ip, 0)) {
            if let Ok(addr) = s.local_addr() {
                candidates.push(Candidate { kind: "host".into(), addr: addr.to_string(), prio: 30000 });
            }
            host_socks.push(s);
        }
    }
    let srflx_sock = std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind udp: {e}"))?;
    srflx_sock.set_nonblocking(true).ok();
    let my_public = local_public_addr(&srflx_sock);
    if let Some(a) = my_public {
        candidates.push(Candidate { kind: "srflx".into(), addr: a.to_string(), prio: 20000 });
    }
    if my_public.is_none() {
        eprintln!("  ⚠ UDP STUN 失败（仅剩 host 候选，或降级 relay）");
    }
    sort_candidates(&mut candidates);

    // ② 发 hello（完整候选列表）→ 等 udp-ack（超时/收到 v1 transit → 降级 relay）
    wormhole
        .send_json(&UdpMsg::Hello {
            candidates,
            fingerprint: fp,
            nonce,
        })
        .await
        .map_err(|e| format!("send udp-hello: {e}"))?;
    let peer_cands = match recv_udp_msg(wormhole, Duration::from_secs(15), "udp-ack").await? {
        Some(UdpMsg::Ack { candidates, .. }) => {
            eprintln!("  ✅ 收到 udp-ack（{} 个候选）", candidates.len());
            candidates
        }
        Some(_) => return Err("收到意外的 UDP 握手消息".into()),
        None => {
            eprintln!("  ⚠ UDP 握手超时，降级 relay");
            return Err("timeout".into());
        }
    };

    // ③ 多端监听：host sockets 直接 listen（同网段免打洞）+ srflx socket 打洞后 listen。
    //    cert/key 需 clone 给多个 quic_listen；endpoint 持有各自 socket 所有权。
    //    关键：打洞（阻塞 ≤3s）放进后台线程，accept 循环立即开始——否则 host 端握手
    //    在打洞期间无人驱动（quinn 不 poll accept 就不推进握手），接收方 2s 超时内
    //    连入的 host 连接会被饿死（实测竞态：第一候选失败、第二候选才成功）。
    let mut endpoints: Vec<quinn::Endpoint> = Vec::new();
    for s in host_socks {
        if let Ok(ep) = crate::commands::quic_link::quic_listen(s, cert.clone(), key.clone_key()).await {
            endpoints.push(ep);
        }
    }
    // srflx 打洞在后台线程执行（需要 'static：srflx_sock/cert/key 移入线程，
    // ack_msg 所需数据克隆）。打洞成功后的 endpoint 经 std channel 送进 accept 循环。
    let (srflx_tx, srflx_rx) = std::sync::mpsc::channel::<quinn::Endpoint>();
    let punch_ready = my_public.is_some();
    if punch_ready {
        let tx = srflx_tx.clone();
        let ack_msg = UdpMsg::Ack { candidates: peer_cands.clone(), nonce };
        let who = who.to_string();
        std::thread::spawn(move || {
            // 复用 STUN 的同一 socket 打洞——NAT 端口映射绑定在 socket 上
            if let Some(direct) = punch_on_socket(srflx_sock, &ack_msg, &who) {
                let ep_fut = crate::commands::quic_link::quic_listen(direct.sock, cert, key);
                // quic_listen 是 async fn，需要小型 executor 驱动；用 async_io 的
                // block_on 跑完再 send（打洞线程本来就是阻塞线程，可接受）
                let ep = async_io::block_on(ep_fut);
                if let Ok(ep) = ep {
                    let _ = tx.send(ep);
                }
            }
        });
    }
    if endpoints.is_empty() && !punch_ready {
        eprintln!("  ⚠ 无可监听端点，降级 relay");
        return Err("no-endpoints".into());
    }

    // ④ accept 循环（20s 超时）：轮询各 endpoint 的非阻塞 accept + srflx channel，
    //    任一成功即用。quinn::Incoming 不可 Clone，每次拿到新的 Incoming 直接
    //    await 驱动握手（带 5s 超时，防恶意连接挂死循环）；握手失败继续轮询。
    //    同时非阻塞监听 wormhole：对方发 Abort（候选全失败）→ 立即降级，不等超时。
    //    20s > 接收方全候选预算（host ≤2s×N + 打洞 3s + 握手 10s），保证 Abort 必达。
    //    哈希等待预算：13s（< 接收方 15s file-meta 窗口，见下方哈希等待注释）。
    let hash_deadline = std::time::Instant::now() + Duration::from_secs(13);
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        // 先收打洞线程送来的 srflx endpoint（非阻塞）
        if let Ok(ep) = srflx_rx.try_recv() {
            eprintln!("  🔗 srflx 打洞成功，加入监听");
            endpoints.push(ep);
        }
        // 非阻塞检查对方是否放弃 UDP（Abort）
        // 安全：接收方仅在全部候选失败时发 Abort（成功则走 QUIC 连接，不发任何消息），
        // 相位对齐不受影响（各端计数独立，见 UdpMsg 注释）。
        if let Some(Ok(Ok(UdpMsg::Abort))) = futures_lite::future::poll_once(wormhole.receive_json::<UdpMsg>()).await {
            eprintln!("  ⚠ 对方放弃 UDP 直连，降级 relay");
            return Err("peer-abort".into());
        }
        for ep in &endpoints {
            if let Some(Some(incoming)) = futures_lite::future::poll_once(ep.accept()).await {
                // S1 安全：握手超时保护（quinn 无握手超时，恶意连接可无限挂死循环）
                let conn = match futures_lite::future::or(
                    async { incoming.await.map_err(|e| format!("入站握手: {e}")) },
                    async {
                        async_io::Timer::after(Duration::from_secs(5)).await;
                        Err("入站握手 5s 超时".into())
                    },
                )
                .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("  ⚠ {e}，继续等待其他候选");
                        continue;
                    }
                };
                // S1 安全：校验对端来源 ∈ 对方通告的候选（防同网段任意设备窃取数据）。
                // - host 路径：对端来源必须精确匹配其通告的 host 地址（同网段直连语义，
                //   来源与通告一一对应——陌生设备不在此集合）
                // - srflx 路径：对端来源 = 打洞学习到的真实地址（对称 NAT 下 ≠ 通告地址，
                //   但只有与我们互发过打洞包的对端才能连入，本身已认证）
                let remote = conn.remote_address();
                let allowed = peer_cands.iter().any(|c| {
                    c.addr.parse::<std::net::SocketAddr>().map(|a| a == remote).unwrap_or(false)
                });
                if !allowed {
                    eprintln!("  ⚠ 拒绝未知来源连接: {remote}（不在对方通告候选内）");
                    continue;
                }
                // Task 3：QUIC 建连后经 wormhole 交换 FileMeta/ChunkStatus（分块续传）。
                // 顺序固定：QUIC 建连 → 发 FileMeta → 收 ChunkStatus（清单回执）→
                // 多流只传缺失块。FileMeta 必须建连后才发——接收方 accept 到连接
                // 才需要文件信息（wormhole 通道双方一直连着，可随时发）。
                // Task 4：FileMeta 前先等后台哈希线程结果（哈希与打洞/accept 并行，
                // 此时通常早已完成；非阻塞轮询防卡 executor）。超时 13s < 接收方 15s
                // file-meta 窗口——哈希超时则本端先降级 relay，FileMeta 绝不迟到
                // 污染对方已降级的 v1 消息流。
                let sha256 = loop {
                    match hash_rx.try_recv() {
                        Ok(Ok(s)) => break s,
                        Ok(Err(e)) => {
                            // 文件读错：哈希失败即发 Abort 通知接收方立即降级（此时
                            // 对方大概率仍在等 file-meta，Abort 被当作非 FileMeta 消息
                            // → 报错 → 降级 relay，消息相位对齐）
                            eprintln!("  ⚠ 文件级 SHA-256 计算失败: {e}");
                            let _ = wormhole.send_json(&UdpMsg::Abort).await;
                            return Err(format!("文件级 SHA-256 计算失败: {e}"));
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            return Err("哈希线程异常退出".into());
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            if std::time::Instant::now() >= hash_deadline {
                                eprintln!("  ⚠ 等待文件哈希超时（>13s），降级 relay");
                                return Err("hash-timeout".into());
                            }
                            async_io::Timer::after(Duration::from_millis(50)).await;
                        }
                    }
                };
                wormhole
                    .send_json(&UdpMsg::FileMeta {
                        name: display_name.to_string(),
                        size: filesize,
                        sha256,
                        chunk_size,
                        chunk_count: plan.len() as u32,
                    })
                    .await
                    .map_err(|e| format!("send file-meta: {e}"))?;
                // 收清单回执 → 缺失块 = 全量块 − 已收块（空回执 = 全量传输）
                let done = match recv_udp_msg(wormhole, Duration::from_secs(15), "chunk-status").await? {
                    Some(UdpMsg::ChunkStatus { done }) => done,
                    Some(_) => return Err("收到意外的 UDP 握手消息".into()),
                    None => return Err("chunk-status 超时".into()),
                };
                let done_set: std::collections::BTreeSet<u32> = done.into_iter().collect();
                let missing = chunked::missing_chunks(plan.len(), &done_set);
                if !done_set.is_empty() {
                    eprintln!("  ♻ 清单回执 {} 块已完成，续传缺失 {} 块", done_set.len(), missing.len());
                }
                let sent = quic_send_chunks(&conn, send_target, &plan, &missing, QUIC_CHUNK_CONCURRENCY, |done, total| {
                    if total > 0 {
                        eprintln!("\r  进度: {}/{} ({:.0}%)", done, total, done as f64 / total as f64 * 100.0);
                    }
                })
                .await?;
                // 传输已完成（对端已确认），drop 全部 endpoint 释放 socket
                drop(endpoints);
                return Ok(sent);
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("  ⚠ 等待对方连接超时，降级 relay");
            return Err("accept-timeout".into());
        }
        async_io::Timer::after(Duration::from_millis(100)).await;
    }
}

/// IPv4 同网段判断（/24）
fn same_subnet_v4(a: std::net::Ipv4Addr, b: std::net::Ipv4Addr, prefix: u32) -> bool {
    let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
    (u32::from(a) & mask) == (u32::from(b) & mask)
}

/// UDP 直连接收（接收方 = QUIC 客户端，ICE-lite 多候选顺序尝试）：
/// 收集自己的 host + srflx 候选 → 回 ack（完整候选列表）→ 按优先级顺序尝试对方候选：
/// host（同网段免打洞，2s 握手）→ srflx（打洞 3s + QUIC 握手 10s）；
/// 全部失败 → Err("punch-fail")，调用方降级 relay。
/// **注意**：指纹不匹配是安全事件（对端证书被篡改/中间人），立即中止返回——
/// 发送方全程用同一张证书，一个候选不匹配则全部不匹配，降级 relay 即绕过校验。
async fn udp_get_path(
    wormhole: &mut Wormhole,
    hello: &UdpMsg,
    who: &str,
    output: &Option<String>,
) -> Result<(String, u64), String> {
    // ① 收集自己的候选（与发送方对称）：
    //    - host：每个本地 IPv4 接口绑一个 socket（ip:0 随机端口），同网段对端免打洞直连
    //    - srflx：0.0.0.0:0 + UDP STUN（同 socket 保证端口映射一致）
    //    注意：STUN 失败也要回 ack（仅 host 候选）——发送方在等 ack，不回会让它
    //    等满 15s 才降级（浪费时间）。空候选让双方立即降级。
    let mut host_socks: Vec<std::net::UdpSocket> = Vec::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    for ip in local_ipv4_addrs() {
        if let Ok(s) = std::net::UdpSocket::bind((ip, 0)) {
            if let Ok(addr) = s.local_addr() {
                candidates.push(Candidate { kind: "host".into(), addr: addr.to_string(), prio: 30000 });
            }
            host_socks.push(s);
        }
    }
    let srflx_sock = std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind udp: {e}"))?;
    srflx_sock.set_nonblocking(true).ok();
    let my_public = local_public_addr(&srflx_sock);
    if my_public.is_none() {
        // L4：接收方 STUN 失败也要有日志（与发送方对称，便于排障）
        eprintln!("  ⚠ UDP STUN 失败（仅 host 候选可用）");
    }
    if let Some(a) = my_public {
        candidates.push(Candidate { kind: "srflx".into(), addr: a.to_string(), prio: 20000 });
    }
    sort_candidates(&mut candidates);
    let nonce = match hello {
        UdpMsg::Hello { nonce, .. } => *nonce,
        _ => return Err("收到意外的 UDP 握手消息".into()),
    };
    let expected_fp = match hello {
        UdpMsg::Hello { fingerprint, .. } => fingerprint.clone(),
        _ => return Err("收到意外的 UDP 握手消息".into()),
    };
    wormhole
        .send_json(&UdpMsg::Ack { candidates, nonce })
        .await
        .map_err(|e| format!("send udp-ack: {e}"))?;

    // ② 按优先级顺序尝试对方候选（host 优先直连，srflx 打洞兜底）
    let peer_cands = match hello {
        UdpMsg::Hello { candidates, .. } => candidates.clone(),
        _ => unreachable!(),
    };
    let mut sorted = peer_cands.clone();
    sort_candidates(&mut sorted);
    let local_ips = local_ipv4_addrs();

    for cand in &sorted {
        match cand.kind.as_str() {
            // host 直连（免打洞）：同网段预筛 + 逐个 host socket 尝试（2s 握手）
            "host" => {
                let peer_ip = cand.addr.split(':').next().unwrap_or("").parse::<std::net::IpAddr>().ok();
                let same = peer_ip
                    .map(|p| match p {
                        std::net::IpAddr::V4(p4) => local_ips.iter().any(|l| same_subnet_v4(*l, p4, 24)),
                        _ => false,
                    })
                    .unwrap_or(false);
                if !same {
                    eprintln!("  ⚠ host 候选 {} 不同网段，跳过", cand.addr);
                    continue;
                }
                let addr: std::net::SocketAddr = match cand.addr.parse() {
                    Ok(a) => a,
                    Err(_) => {
                        // S2：坏地址也是失败出口——发 Abort 通知发送方，避免它空等 accept 超时
                        eprintln!("  ⚠ 坏 host 地址: {}", cand.addr);
                        let _ = wormhole.send_json(&UdpMsg::Abort).await;
                        return Err("punch-fail".into());
                    }
                };
                // M3：只尝试与候选同网段的 socket（跨网段 socket 必超时，浪费 2s/次）
                let peer_v4: Option<std::net::Ipv4Addr> = match addr.ip() {
                    std::net::IpAddr::V4(v4) => Some(v4),
                    _ => None,
                };
                for s in &host_socks {
                    let sock_local = match s.local_addr() {
                        Ok(a) => a,
                        Err(_) => continue,
                    };
                    let same_sock_subnet = match (peer_v4, sock_local.ip()) {
                        (Some(p), std::net::IpAddr::V4(l)) => same_subnet_v4(l, p, 24),
                        _ => false,
                    };
                    if !same_sock_subnet {
                        continue;
                    }
                    // quic_connect 消费 socket 所有权 → 每个尝试 try_clone 独立 socket
                    let sock = s.try_clone().map_err(|e| format!("clone: {e}"))?;
                    eprintln!("  🔗 尝试 host 直连 {} → {}", sock.local_addr().map(|a| a.to_string()).unwrap_or_default(), addr);
                    match crate::commands::quic_link::quic_connect(sock, addr, expected_fp.clone(), Duration::from_secs(2)).await {
                        Ok((ep, conn)) => {
                            let r = quic_recv_resume(&conn, wormhole, output, |d, t| {
                                if t > 0 { eprintln!("\r  进度: {}/{} ({:.0}%)", d, t, d as f64 / t as f64 * 100.0); }
                            }).await;
                            conn.close(0u32.into(), b"done");
                            drop(ep);
                            match r {
                                Ok(v) => {
                                    eprintln!("  ✅ host 直连成功");
                                    return Ok(v);
                                }
                                Err(e) => eprintln!("  ⚠ host 直连收文件失败: {e}"),
                            }
                        }
                        Err(e) if e.contains("fingerprint") => return Err(e), // 安全事件：中止，绝不降级 relay
                        Err(e) => { eprintln!("  ⚠ host 直连失败: {e}"); }
                    }
                }
            }
            // srflx 打洞（最后手段）：打洞 3s + QUIC 握手 10s
            "srflx" => {
                let peer_msg = UdpMsg::Hello { candidates: peer_cands.clone(), fingerprint: expected_fp.clone(), nonce };
                eprintln!("  🔗 尝试 srflx 打洞 {}", cand.addr);
                match punch_on_socket(srflx_sock.try_clone().map_err(|e| format!("clone: {e}"))?, &peer_msg, who) {
                    Some(direct) => {
                        match crate::commands::quic_link::quic_connect(direct.sock, direct.peer, expected_fp.clone(), Duration::from_secs(10)).await {
                            Ok((ep, conn)) => {
                                let r = quic_recv_resume(&conn, wormhole, output, |d, t| {
                                    if t > 0 { eprintln!("\r  进度: {}/{} ({:.0}%)", d, t, d as f64 / t as f64 * 100.0); }
                                }).await;
                                conn.close(0u32.into(), b"done");
                                drop(ep);
                                match r {
                                    Ok(v) => {
                                        eprintln!("  ✅ srflx 打洞直连成功");
                                        return Ok(v);
                                    }
                                    Err(e) => eprintln!("  ⚠ srflx 直连收文件失败: {e}"),
                                }
                            }
                            Err(e) if e.contains("fingerprint") => return Err(e), // 安全事件：中止，绝不降级 relay
                            Err(e) => { eprintln!("  ⚠ quic_connect 失败: {e}"); }
                        }
                    }
                    None => { eprintln!("  ⚠ srflx 打洞失败"); }
                }
            }
            _ => { /* host6/srflx6 预留 */ }
        }
    }
    // 全部失败：通知发送方放弃 UDP（它收到 Abort 立即降级，不必等 accept 超时 15s）。
    // 相位对称：双方各多收发 1 条消息，v1 降级仍对齐（见 UdpMsg 注释）。
    let _ = wormhole.send_json(&UdpMsg::Abort).await;
    Err("punch-fail".into()) // 全部失败 → 调用方降级 relay
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

        // relay 兜底：v3 对端 → 多线程分块（partial + 清单跨路径复用，只传 missing）；
        // 旧端（无 v3 能力）→ v1 单流整文件传输（旧行为，协商层已隔离）
        if peer_supports_udp(&wormhole) {
            let sent = match relay_send_resume(
                &mut wormhole,
                send_target.as_str(),
                &display_name,
                |done, total| {
                    if total > 0 {
                        eprintln!("\r  进度: {}/{} ({:.0}%)", done, total, done as f64 / total as f64 * 100.0);
                    }
                },
            )
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    // 通知接收方立即放弃（收满的块仍会正常收尾——done 满优先于 Abort）
                    let _ = wormhole.send_json(&UdpMsg::Abort).await;
                    return Err(format!("relay 分块传输失败: {e}"));
                }
            };
            println!("\n  ✅ relay 分块传输完成，校验通过（{} 字节）", sent);
            Ok((code, peer_key, sent))
        } else {
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
            )
            .await;

            match res {
                Ok(()) => {
                    let sent = total_bytes.load(std::sync::atomic::Ordering::Relaxed);
                    println!("\n  ✅ 传输完成，校验通过（{} 字节）", sent);
                    Ok((code, peer_key, sent))
                }
                Err(e) => Err(format!("传输失败: {}", e)),
            }
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

        // v3 对端：先尝试 UDP 打洞（FAN_NO_UDP=1 时跳过），失败/禁用 → relay 分块；
        // 旧端（无 v3 能力）→ 下方 v1 单流整文件传输（协商层已隔离）。
        // **注意**：relay 分块不依赖 FAN_NO_UDP——该开关只跳过 UDP 相位，双方
        // 仍走 v3 relay 分块（消息流一致：FileMeta/ChunkStatus/RelayChunk）。
        if peer_supports_udp(&wormhole) {
            // v3 relay 分块的成功/失败收尾（各入口共用）；peer_key 以参数传入，
            // 避免闭包借用与 Hello 分支的移动冲突
            let finish_relay = |r: Result<(String, u64), String>, pk: &str| -> Result<(String, u64), String> {
                match r {
                    Ok((file_name, received)) => {
                        println!("\n  ✅ relay 分块接收完成，SHA-256 校验通过（{} 字节）", received);
                        println!("  已保存: {}", file_name);
                        Ok((pk.to_string(), received))
                    }
                    Err(e) => Err(format!("relay 分块接收失败: {e}")),
                }
            };
            if !udp_disabled() {
                eprintln!("  🔎 对端支持 UDP 打洞，等待对方 UDP 握手…");
                // 接收方：限时等 udp-hello（旧版发送方不会发 hello，超时即降级）。
                // v3 relay 分块下 FileMeta 也可在 hello 阶段直接收到（单边
                // FAN_NO_UDP 等场景）——已消费的消息不重收，直接进入 relay 分块。
                match recv_udp_msg(&mut wormhole, Duration::from_secs(15), "udp-hello").await {
                    Ok(Some(hello @ UdpMsg::Hello { .. })) => {
                        // UDP 路径：目标路径在 quic_recv_resume 内部解析（含文件名）
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
                        // UDP 失败 → v3 relay 分块（消息流未错位：FileMeta 是对方下一条）
                        return finish_relay(relay_get_result(&mut wormhole, &output, None).await, &peer_key);
                    }
                    Ok(Some(meta @ UdpMsg::FileMeta { .. })) => {
                        // 对方直接走 relay 分块（单边 FAN_NO_UDP 等）：FileMeta 已收，
                        // 不再重复接收（相位计数保持对称）
                        return finish_relay(relay_get_result(&mut wormhole, &output, Some(meta)).await, &peer_key);
                    }
                    Ok(Some(_)) | Ok(None) => {
                        // 收到 Abort（对方已降级）/ hello 超时（对方已降级或走 v1）：
                        // 继续等 FileMeta（v3 relay 分块）；v1 transit 由下方请求解析
                        // 失败兜底
                        return finish_relay(relay_get_result(&mut wormhole, &output, None).await, &peer_key);
                    }
                    Err(_) => {
                        // 收到非 UdpMsg（v1 transit）→ 对方走 v1 → 下方 v1 路径
                    }
                }
            } else {
                // FAN_NO_UDP=1：跳过 UDP 相位，直接走 relay 分块（等 FileMeta）
                return finish_relay(relay_get_result(&mut wormhole, &output, None).await, &peer_key);
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

    #[test]
    fn same_subnet_v4_detects_subnet() {
        let a: std::net::Ipv4Addr = "10.98.103.5".parse().unwrap();
        let b: std::net::Ipv4Addr = "10.98.103.200".parse().unwrap();
        let c: std::net::Ipv4Addr = "10.99.0.1".parse().unwrap();
        assert!(same_subnet_v4(a, b, 24));
        assert!(!same_subnet_v4(a, c, 24));
    }

    #[test]
    fn udp_msg_candidates_roundtrip() {
        let m = UdpMsg::Hello {
            candidates: vec![
                Candidate { kind: "host".into(), addr: "10.0.0.5:3000".into(), prio: 30000 },
                Candidate { kind: "srflx".into(), addr: "1.2.3.4:4000".into(), prio: 20000 },
            ],
            fingerprint: "f".repeat(64),
            nonce: 42,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: UdpMsg = serde_json::from_str(&json).unwrap();
        match back {
            UdpMsg::Hello { candidates, fingerprint, nonce } => {
                assert_eq!(candidates.len(), 2);
                assert_eq!(candidates[0].kind, "host");
                assert_eq!(fingerprint.len(), 64);
                assert_eq!(nonce, 42);
            }
            _ => panic!("应为 Hello"),
        }
    }

    /// 多流并发分块回环：同机双 endpoint，发送方 2 worker 并行传 2 块（8MB 文件
    /// 4MB 块），接收方每流一个 task 并发收块写 partial。断言：收到 2 块、partial
    /// 内容与原文件 SHA-256 一致、进度回调累计到总字节。
    /// 方向与生产一致：发送方 = QUIC 服务端（quic_listen），接收方 = QUIC 客户端
    /// （quic_connect）。参考 quic_link.rs 的 quic_stream_roundtrip_one_byte 模式。
    #[test]
    fn quic_multi_stream_parallel_chunks() {
        let tmp = std::env::temp_dir().join("fan-chunk-test.bin");
        let data = vec![0xABu8; 8 * 1024 * 1024];
        std::fs::write(&tmp, &data).unwrap();
        let tmp_str = tmp.to_string_lossy().to_string();
        // partial/清单键：用唯一 hash，测试结束 clear_manifest 清理
        let hash16 = "test-hash-1";
        chunked::clear_manifest(hash16);

        let (cert, key, fp) = crate::commands::quic_link::gen_cert_with_fingerprint();
        let server_sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        server_sock.set_nonblocking(true).unwrap();
        let addr = server_sock.local_addr().unwrap();
        let client_sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        client_sock.set_nonblocking(true).unwrap();

        // 进度回调累计（发送/接收各一份，断言最后一次 = 总字节）
        let snd_progress: std::sync::Arc<std::sync::Mutex<Vec<(u64, u64)>>> = Default::default();
        let rcv_progress: std::sync::Arc<std::sync::Mutex<Vec<(u64, u64)>>> = Default::default();

        let result = async_std::task::block_on(async {
            let server = crate::commands::quic_link::quic_listen(server_sock, cert, key).await.expect("server endpoint");
            // 发送方（QUIC 服务端）：worker 池并发传块
            let srv = futures_lite::future::or(
                async {
                    let incoming = server.accept().await.ok_or("server accept 超时")?;
                    let conn = incoming.await.map_err(|e| format!("incoming handshake: {e}"))?;
                    let plan = chunked::chunk_plan(8 * 1024 * 1024, chunked::DEFAULT_CHUNK_SIZE);
                    assert_eq!(plan.len(), 2, "8MB/4MB 应为 2 块");
                    let missing: Vec<u32> = (0..plan.len() as u32).collect();
                    let p = snd_progress.clone();
                    let sent = quic_send_chunks(&conn, &tmp_str, &plan, &missing, 2,
                        move |d, t| { p.lock().unwrap().push((d, t)); }).await?;
                    assert_eq!(sent, 8 * 1024 * 1024, "发送字节数应等于文件大小");
                    conn.close(0u32.into(), b"test done");
                    let _ = conn.closed().await;
                    Ok::<(), String>(())
                },
                async {
                    async_io::Timer::after(Duration::from_secs(30)).await;
                    Err("server 侧 30s 超时".to_string())
                },
            );
            // 接收方（QUIC 客户端）：并发收块写 partial + 更新清单
            let cli = futures_lite::future::or(
                async {
                    let (endpoint, conn) = crate::commands::quic_link::quic_connect(client_sock, addr, fp, Duration::from_secs(10)).await?;
                    let p = rcv_progress.clone();
                    let (chunks, bytes) = quic_recv_chunks(&conn, hash16, "fan-chunk-test.bin",
                        8 * 1024 * 1024, chunked::DEFAULT_CHUNK_SIZE, 2, &[],
                        move |d, t| { p.lock().unwrap().push((d, t)); }).await?;
                    assert_eq!(chunks, 2, "应收满 2 块");
                    assert_eq!(bytes, 8 * 1024 * 1024, "收到字节数应等于文件大小");
                    conn.close(0u32.into(), b"test done");
                    drop(endpoint);
                    Ok::<(), String>(())
                },
                async {
                    async_io::Timer::after(Duration::from_secs(30)).await;
                    Err("client 侧 30s 超时".to_string())
                },
            );
            let (srv_res, cli_res) = futures_lite::future::zip(srv, cli).await;
            srv_res?;
            cli_res
        });

        // 断言 partial 文件内容与原文件一致（块按 offset 写入 = 完整文件）
        let part = chunked::partial_path(hash16);
        let part_matches = part.exists()
            && std::fs::read(&part).map(|d| d == data).unwrap_or(false);
        // 断言进度回调最后一次 = (总字节, 总字节)
        let snd_ok = snd_progress.lock().unwrap().last() == Some(&(8 * 1024 * 1024, 8 * 1024 * 1024));
        let rcv_ok = rcv_progress.lock().unwrap().last() == Some(&(8 * 1024 * 1024, 8 * 1024 * 1024));
        // 清理（partial + 清单 + 临时文件）
        chunked::clear_manifest(hash16);
        let _ = std::fs::remove_file(&tmp);

        assert!(result.is_ok(), "多流并发分块传输应成功: {result:?}");
        assert!(part_matches, "partial 文件内容应与原文件一致（块级 SHA-256 + offset 写入）");
        assert!(snd_ok, "发送方进度应累计到总字节: {:?}", snd_progress.lock().unwrap());
        assert!(rcv_ok, "接收方进度应累计到总字节: {:?}", rcv_progress.lock().unwrap());
    }

    /// 单块文件（3MB < 4MB 块）统一走分块路径：FileMeta/ChunkStatus/1 块完整传输。
    /// 验证 chunk_count=1 时 chunks 路径与多块路径行为一致（回环 + 文件级校验 +
    /// finalize rename）。协议顺序与生产一致：QUIC 建连 → 发 FileMeta → 收
    /// ChunkStatus（空）→ 传唯一一块 → 接收方文件级 SHA-256 校验收尾。
    #[test]
    fn quic_single_chunk_file_roundtrip() {
        let tmp = std::env::temp_dir().join("fan-single-chunk.bin");
        let data = vec![0x5Au8; 3 * 1024 * 1024];
        std::fs::write(&tmp, &data).unwrap();
        let tmp_str = tmp.to_string_lossy().to_string();
        let sha = sha256_file(&tmp).unwrap();
        let hash16: String = sha.chars().take(16).collect();
        let output = std::env::temp_dir().join("fan-single-chunk-out.bin");
        let _ = std::fs::remove_file(&output);
        chunked::clear_manifest(&hash16); // 防上次运行残留
        let file_size = 3 * 1024 * 1024u64;

        let (cert, key, fp) = crate::commands::quic_link::gen_cert_with_fingerprint();
        let server_sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        server_sock.set_nonblocking(true).unwrap();
        let addr = server_sock.local_addr().unwrap();
        let client_sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        client_sock.set_nonblocking(true).unwrap();
        let (meta_tx, meta_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let (status_tx, status_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let output_str = output.to_string_lossy().to_string();

        let result = async_std::task::block_on(async {
            let server = crate::commands::quic_link::quic_listen(server_sock, cert, key)
                .await.expect("server endpoint");
            // 发送方：FileMeta（chunk_count=1）→ 等 ChunkStatus（空）→ 传唯一一块
            let srv = futures_lite::future::or(
                async {
                    let incoming = server.accept().await.ok_or("server accept 超时")?;
                    let conn = incoming.await.map_err(|e| format!("incoming handshake: {e}"))?;
                    let plan = chunked::chunk_plan(file_size, chunked::DEFAULT_CHUNK_SIZE);
                    assert_eq!(plan.len(), 1, "3MB/4MB 应为 1 块");
                    let meta = UdpMsg::FileMeta {
                        name: "fan-single-chunk.bin".into(),
                        size: file_size,
                        sha256: sha.clone(),
                        chunk_size: chunked::DEFAULT_CHUNK_SIZE,
                        chunk_count: 1,
                    };
                    let meta_json = serde_json::to_vec(&meta).map_err(|e| format!("FileMeta 序列化: {e}"))?;
                    meta_tx.send(meta_json).map_err(|e| format!("meta 转发: {e}"))?;
                    let status_json = chan_recv(&status_rx).await?;
                    let status: UdpMsg = serde_json::from_slice(&status_json)
                        .map_err(|e| format!("ChunkStatus 解析: {e}"))?;
                    let done: std::collections::BTreeSet<u32> = match status {
                        UdpMsg::ChunkStatus { done } => done.into_iter().collect(),
                        _ => return Err("应收到 ChunkStatus".into()),
                    };
                    assert!(done.is_empty(), "首次传输应无已收块");
                    let missing = chunked::missing_chunks(plan.len(), &done);
                    assert_eq!(missing, vec![0], "单块文件应只缺第 0 块");
                    let sent = quic_send_chunks(&conn, &tmp_str, &plan, &missing, 2, |_, _| {}).await?;
                    assert_eq!(sent, 3 * 1024 * 1024, "单块应传完整文件字节数");
                    conn.close(0u32.into(), b"single done");
                    let _ = conn.closed().await;
                    Ok::<(), String>(())
                },
                async {
                    async_io::Timer::after(Duration::from_secs(30)).await;
                    Err("server 30s 超时".to_string())
                },
            );
            // 接收方：收 FileMeta → 回 ChunkStatus → 收 1 块 → 文件级校验 + 收尾
            let cli = futures_lite::future::or(
                async {
                    let (ep, conn) = crate::commands::quic_link::quic_connect(
                        client_sock, addr, fp, Duration::from_secs(10)).await?;
                    let meta_json = chan_recv(&meta_rx).await?;
                    let meta: UdpMsg = serde_json::from_slice(&meta_json)
                        .map_err(|e| format!("FileMeta 解析: {e}"))?;
                    let (name, size, s, chunk_size, chunk_count) = match meta {
                        UdpMsg::FileMeta { name, size, sha256, chunk_size, chunk_count } =>
                            (name, size, sha256, chunk_size, chunk_count),
                        _ => return Err("应收到 FileMeta".into()),
                    };
                    assert_eq!(chunk_count, 1, "单块文件 chunk_count 应为 1");
                    // 与 quic_recv_resume 相同的自洽校验（Task 4 C1）
                    let expect_chunks = chunked::chunk_plan(size, chunk_size).len() as u32;
                    assert_eq!(chunk_count, expect_chunks, "chunk_count 应与分块计划一致");
                    let hash16: String = s.chars().take(16).collect();
                    let st = UdpMsg::ChunkStatus { done: Vec::new() };
                    let st_json = serde_json::to_vec(&st).map_err(|e| format!("ChunkStatus 序列化: {e}"))?;
                    status_tx.send(st_json).map_err(|e| format!("status 转发: {e}"))?;
                    let (chunks, bytes) = quic_recv_chunks(&conn, &hash16, &name, size,
                        chunk_size, chunk_count, &[], |_, _| {}).await?;
                    assert_eq!(chunks, 1, "应收满 1 块");
                    assert_eq!(bytes, 3 * 1024 * 1024, "应收完整文件字节数");
                    let target = finalize_partial(&hash16, &name, &s, &Some(output_str.clone()))?;
                    assert_eq!(target, output, "应输出到 --output 指定路径");
                    conn.close(0u32.into(), b"done");
                    drop(ep);
                    Ok::<(), String>(())
                },
                async {
                    async_io::Timer::after(Duration::from_secs(30)).await;
                    Err("client 30s 超时".to_string())
                },
            );
            let (srv_res, cli_res) = futures_lite::future::zip(srv, cli).await;
            srv_res?;
            cli_res
        });

        assert!(result.is_ok(), "单块分块传输应成功: {result:?}");
        assert_eq!(sha256_file(&output).unwrap(), sha, "单块传输结果应与原文件一致（文件级 SHA-256）");
        assert!(chunked::load_manifest(&hash16).is_none(), "收尾后清单应清理");
        assert!(!chunked::partial_path(&hash16).exists(), "收尾后 partial 应移除");
        // 清理
        chunked::clear_manifest(&hash16);
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&output);
    }

    /// 后台哈希通道（Task 4 A）：模拟 udp_send_path 的哈希线程启动方式——spawn
    /// 线程算 SHA-256 经 mpsc 送回，主线程 try_recv 轮询（与 accept 循环内等哈希
    /// 的写法一致，Empty 分支不阻塞）。断言：结果与同步计算一致、轮询期间主线程
    /// 不被卡死、失败路径（文件不存在）经通道收到 Err（对应发 Abort 降级分支）。
    #[test]
    fn background_hash_thread_channel_roundtrip() {
        // 中等文件（8MB）：哈希线程与主线程并行，模拟大文件哈希不阻塞 hello 的场景
        let tmp = std::env::temp_dir().join("fan-bg-hash.bin");
        let data = vec![0x3Cu8; 8 * 1024 * 1024];
        std::fs::write(&tmp, &data).unwrap();
        let expected = sha256_file(&tmp).unwrap();

        let (hash_tx, hash_rx) = std::sync::mpsc::channel::<Result<String, String>>();
        let hash_path = tmp.to_string_lossy().to_string();
        std::thread::spawn(move || {
            let _ = hash_tx.send(sha256_file(std::path::Path::new(&hash_path)));
        });
        // try_recv 轮询（与 udp_send_path 等哈希逻辑同构）：Empty 分支立即重试，
        // 主线程绝不阻塞；哈希完成后收到正确结果
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let got = loop {
            match hash_rx.try_recv() {
                Ok(Ok(s)) => break s,
                Ok(Err(e)) => panic!("哈希线程不应失败: {e}"),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => panic!("哈希线程异常退出"),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    assert!(std::time::Instant::now() < deadline, "等后台哈希超时");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        };
        assert_eq!(got, expected, "后台哈希结果应与同步计算一致");

        // 失败路径：文件不存在 → 通道收到 Err（udp_send_path 据此发 Abort 降级）
        let (hash_tx, hash_rx) = std::sync::mpsc::channel::<Result<String, String>>();
        let missing = std::env::temp_dir().join("fan-bg-hash-missing.bin");
        std::thread::spawn(move || {
            let _ = hash_tx.send(sha256_file(std::path::Path::new(&missing)));
        });
        let res = hash_rx.recv_timeout(Duration::from_secs(30)).expect("哈希线程应送回结果");
        assert!(res.is_err(), "不存在的文件应返回 Err（触发 Abort 降级）");

        let _ = std::fs::remove_file(&tmp);
    }

    /// FileMeta/ChunkStatus 消息序列化 roundtrip（kebab-case 标签 + 字段重命名）
    #[test]
    fn udp_msg_file_meta_chunk_status_roundtrip() {
        let meta = UdpMsg::FileMeta {
            name: "dataset.tar".into(),
            size: 10 * 1024 * 1024,
            sha256: "ab".repeat(32),
            chunk_size: chunked::DEFAULT_CHUNK_SIZE,
            chunk_count: 3,
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("\"file-meta\""), "kebab-case 标签应为 file-meta: {json}");
        // 注：serde 的 enum rename_all 只影响变体名（type 标签），结构体字段保持
        // 原样（chunk_size）；双方用同一 derive，线上格式一致即可
        assert!(json.contains("\"chunk_size\""), "字段名保持 snake_case: {json}");
        let back: UdpMsg = serde_json::from_str(&json).unwrap();
        match back {
            UdpMsg::FileMeta { name, size, sha256, chunk_size, chunk_count } => {
                assert_eq!(name, "dataset.tar");
                assert_eq!(size, 10 * 1024 * 1024);
                assert_eq!(sha256.len(), 64);
                assert_eq!(chunk_size, chunked::DEFAULT_CHUNK_SIZE);
                assert_eq!(chunk_count, 3);
            }
            _ => panic!("应为 FileMeta"),
        }
        let st = UdpMsg::ChunkStatus { done: vec![0, 1, 3] };
        let json = serde_json::to_string(&st).unwrap();
        assert!(json.contains("\"chunk-status\""), "kebab-case 标签应为 chunk-status: {json}");
        let back: UdpMsg = serde_json::from_str(&json).unwrap();
        match back {
            UdpMsg::ChunkStatus { done } => assert_eq!(done, vec![0, 1, 3]),
            _ => panic!("应为 ChunkStatus"),
        }
    }

    /// RelayChunk 消息序列化 roundtrip（kebab-case 标签 + 字段）：relay 分块配对
    /// 码经主通道发送，接收方用 code 发起 transfer get 收块。
    #[test]
    fn udp_msg_relay_chunk_roundtrip() {
        let m = UdpMsg::RelayChunk {
            code: "1-breeze-kitten".into(),
            index: 2,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"relay-chunk\""), "kebab-case 标签应为 relay-chunk: {json}");
        assert!(json.contains("1-breeze-kitten"), "应含块配对码: {json}");
        let back: UdpMsg = serde_json::from_str(&json).unwrap();
        match back {
            UdpMsg::RelayChunk { code, index } => {
                assert_eq!(code, "1-breeze-kitten");
                assert_eq!(index, 2);
            }
            _ => panic!("应为 RelayChunk"),
        }
        // 打洞函数应把 RelayChunk 视为非打洞阶段消息（无打洞意图）
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let direct = punch_on_socket(sock, &m, "sender");
        assert!(direct.is_none(), "RelayChunk 不应触发打洞");
    }

    /// OffsetWriter：一次 accept 的块数据流写入 partial 对应 offset。多块并发
    /// 交错写（共享句柄 + 每次写前重新 seek 到 offset+已写字节），不同块区域
    /// 互不覆盖；只 seek 一次的版本会被共享游标带偏（回归测试）。
    #[test]
    fn offset_writer_writes_at_chunk_offset() {
        use futures_lite::io::AsyncWriteExt;
        let path = std::env::temp_dir().join("fan-offset-writer.bin");
        let _ = std::fs::remove_file(&path);
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&path)
            .unwrap();
        let file_arc: std::sync::Arc<std::sync::Mutex<std::fs::File>> =
            std::sync::Arc::new(std::sync::Mutex::new(f));
        // 块 0 在 offset 0，块 1 在 offset 4：并发写不覆盖（共享句柄 + Mutex 串行化）
        // 两个块区域不重叠（块 0 在 offset 0、块 1 在 offset 10），交错写入模拟
        // 并发 worker 共享句柄——每次写前必须重新 seek 到自己的 offset + 已写
        // 字节，否则会被对方移动的共享游标带偏（写错位置）。
        let w1 = OffsetWriter { file: file_arc.clone(), offset: 0, written: std::cell::Cell::new(0) };
        let w2 = OffsetWriter { file: file_arc.clone(), offset: 10, written: std::cell::Cell::new(0) };
        async_std::task::block_on(async {
            let mut w1 = w1;
            let mut w2 = w2;
            w1.write_all(&[b'A'; 2]).await.unwrap();
            w2.write_all(&[b'B'; 2]).await.unwrap();
            w1.write_all(&[b'C'; 2]).await.unwrap();
            w2.write_all(&[b'D'; 2]).await.unwrap();
            w1.write_all(&[b'E'; 2]).await.unwrap();
        });
        let data = std::fs::read(&path).unwrap();
        assert_eq!(&data[0..6], &[b'A', b'A', b'C', b'C', b'E', b'E'], "块 0 连续写应按序落在 offset 0（不被共享游标带偏）");
        assert_eq!(&data[10..14], &[b'B', b'B', b'D', b'D'], "块 1 连续写应落在其 offset");
        assert!(data[6..10].iter().all(|b| *b == 0), "两块之间不应有漂移写入");
        assert_eq!(data.len(), 14);
        let _ = std::fs::remove_file(&path);
    }

    /// 模拟 wormhole 的消息通道：std mpsc + try_recv 轮询（不阻塞 async executor）
    async fn chan_recv(rx: &std::sync::mpsc::Receiver<Vec<u8>>) -> Result<Vec<u8>, String> {
        loop {
            if let Ok(v) = rx.try_recv() {
                return Ok(v);
            }
            async_io::Timer::after(Duration::from_millis(10)).await;
        }
    }

    /// 断线续传回环（同机双 endpoint × 2 轮）：
    /// 第 1 轮模拟断线：发送方只传前 2 块（清单 done=[0,1]，partial = 前 8MB）后关闭
    /// 连接，接收方 accept 循环发现连接关闭（4 块未收满）→ 断线错误——清单已持久化。
    /// 第 2 轮完整续传流程：发送方算文件级 SHA-256 → FileMeta → 接收方查清单回
    /// ChunkStatus{done:[0,1]} → 发送方只传缺失块 [2,3]（进度止于 8MB 而非 16MB）→
    /// 接收方收满后文件级校验 → rename → 清理清单。断言最终文件 == 原文件。
    #[test]
    fn quic_chunked_resume_skips_done_chunks() {
        let tmp = std::env::temp_dir().join("fan-resume-test.bin");
        let data = vec![0xCDu8; 16 * 1024 * 1024];
        std::fs::write(&tmp, &data).unwrap();
        let tmp_str = tmp.to_string_lossy().to_string();
        let sha = sha256_file(&tmp).unwrap();
        let hash16: String = sha.chars().take(16).collect();
        let output = std::env::temp_dir().join("fan-resume-out.bin");
        let _ = std::fs::remove_file(&output);
        chunked::clear_manifest(&hash16); // 防上次运行残留
        let file_size = 16 * 1024 * 1024u64;

        let (cert, key, fp) = crate::commands::quic_link::gen_cert_with_fingerprint();

        // ---------- 第 1 轮：模拟断线（发送方只传前 2 块后关闭连接） ----------
        let server_sock1 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        server_sock1.set_nonblocking(true).unwrap();
        let addr1 = server_sock1.local_addr().unwrap();
        let client_sock1 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        client_sock1.set_nonblocking(true).unwrap();

        let r1 = async_std::task::block_on(async {
            let server1 = crate::commands::quic_link::quic_listen(server_sock1, cert.clone(), key.clone_key())
                .await.expect("server1 endpoint");
            let srv = futures_lite::future::or(
                async {
                    let incoming = server1.accept().await.ok_or("server1 accept 超时")?;
                    let conn = incoming.await.map_err(|e| format!("incoming handshake: {e}"))?;
                    let plan = chunked::chunk_plan(file_size, chunked::DEFAULT_CHUNK_SIZE);
                    assert_eq!(plan.len(), 4, "16MB/4MB 应为 4 块");
                    // 模拟断线：只传前 2 块，传完即断
                    let sent = quic_send_chunks(&conn, &tmp_str, &plan, &[0, 1], 2, |_, _| {}).await?;
                    assert_eq!(sent, 8 * 1024 * 1024, "首轮应只传前 8MB");
                    conn.close(0u32.into(), b"disconnect");
                    let _ = conn.closed().await;
                    Ok::<(), String>(())
                },
                async {
                    async_io::Timer::after(Duration::from_secs(30)).await;
                    Err("server1 30s 超时".to_string())
                },
            );
            let cli = futures_lite::future::or(
                async {
                    let (ep, conn) = crate::commands::quic_link::quic_connect(
                        client_sock1, addr1, fp.clone(), Duration::from_secs(10)).await?;
                    // 收满 2 块后连接关闭 → 断线路径（done=2 < 4 未收满 → Err）
                    let r = quic_recv_chunks(&conn, &hash16, "fan-resume-test.bin",
                        file_size, chunked::DEFAULT_CHUNK_SIZE, 4, &[], |_, _| {}).await;
                    conn.close(0u32.into(), b"done");
                    drop(ep);
                    r.map(|_| ()).map_err(|e| format!("接收方应报断线: {e}"))
                },
                async {
                    async_io::Timer::after(Duration::from_secs(30)).await;
                    Err("client1 30s 超时".to_string())
                },
            );
            let (srv_res, cli_res) = futures_lite::future::zip(srv, cli).await;
            srv_res?;
            cli_res
        });
        assert!(r1.is_err(), "第 1 轮应模拟断线失败（未收满 4 块）: {r1:?}");
        // 断线后清单应已持久化 done=[0,1]
        let m = chunked::load_manifest(&hash16).expect("断线后清单应存在");
        assert_eq!(m.done, vec![0, 1], "断线后清单应记录前两块");
        assert_eq!(m.file_size, file_size);
        assert_eq!(m.chunk_size, chunked::DEFAULT_CHUNK_SIZE);

        // ---------- 第 2 轮：完整续传流程（FileMeta/ChunkStatus 经通道模拟 wormhole） ----------
        let server_sock2 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        server_sock2.set_nonblocking(true).unwrap();
        let addr2 = server_sock2.local_addr().unwrap();
        let client_sock2 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        client_sock2.set_nonblocking(true).unwrap();
        let (meta_tx, meta_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let (status_tx, status_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let snd_progress: std::sync::Arc<std::sync::Mutex<Vec<(u64, u64)>>> = Default::default();
        let rcv_progress: std::sync::Arc<std::sync::Mutex<Vec<(u64, u64)>>> = Default::default();
        let output_str = output.to_string_lossy().to_string();

        let result = async_std::task::block_on(async {
            let server2 = crate::commands::quic_link::quic_listen(server_sock2, cert, key)
                .await.expect("server2 endpoint");
            // 发送方：算文件级 SHA-256 → 分块 → FileMeta → 等 ChunkStatus → 只传缺失块
            let srv = futures_lite::future::or(
                async {
                    let incoming = server2.accept().await.ok_or("server2 accept 超时")?;
                    let conn = incoming.await.map_err(|e| format!("incoming handshake: {e}"))?;
                    let s = sha256_file(std::path::Path::new(&tmp_str))?;
                    assert_eq!(&s[..16], hash16, "hash16 应为文件级 SHA-256 前 16 字符");
                    let plan = chunked::chunk_plan(file_size, chunked::DEFAULT_CHUNK_SIZE);
                    let meta = UdpMsg::FileMeta {
                        name: "fan-resume-test.bin".into(),
                        size: file_size,
                        sha256: s,
                        chunk_size: chunked::DEFAULT_CHUNK_SIZE,
                        chunk_count: plan.len() as u32,
                    };
                    let meta_json = serde_json::to_vec(&meta).map_err(|e| format!("FileMeta 序列化: {e}"))?;
                    meta_tx.send(meta_json).map_err(|e| format!("meta 转发: {e}"))?;
                    // 收 ChunkStatus（清单回执）
                    let status_json = chan_recv(&status_rx).await?;
                    let status: UdpMsg = serde_json::from_slice(&status_json)
                        .map_err(|e| format!("ChunkStatus 解析: {e}"))?;
                    let done: std::collections::BTreeSet<u32> = match status {
                        UdpMsg::ChunkStatus { done } => done.into_iter().collect(),
                        _ => return Err("应收到 ChunkStatus".into()),
                    };
                    let missing = chunked::missing_chunks(plan.len(), &done);
                    assert_eq!(missing, vec![2, 3], "清单回执后应只缺 2、3 块");
                    let p = snd_progress.clone();
                    let sent = quic_send_chunks(&conn, &tmp_str, &plan, &missing, 2,
                        move |d, t| { p.lock().unwrap().push((d, t)); }).await?;
                    assert_eq!(sent, 8 * 1024 * 1024, "续传应只传 8MB");
                    conn.close(0u32.into(), b"resume done");
                    let _ = conn.closed().await;
                    Ok::<(), String>(())
                },
                async {
                    async_io::Timer::after(Duration::from_secs(30)).await;
                    Err("server2 30s 超时".to_string())
                },
            );
            // 接收方：收 FileMeta → 查清单 → 回 ChunkStatus → 收缺失块 → 校验 + 收尾
            let cli = futures_lite::future::or(
                async {
                    let (ep, conn) = crate::commands::quic_link::quic_connect(
                        client_sock2, addr2, fp.clone(), Duration::from_secs(10)).await?;
                    let meta_json = chan_recv(&meta_rx).await?;
                    let meta: UdpMsg = serde_json::from_slice(&meta_json)
                        .map_err(|e| format!("FileMeta 解析: {e}"))?;
                    let (name, size, s, chunk_size, chunk_count) = match meta {
                        UdpMsg::FileMeta { name, size, sha256, chunk_size, chunk_count } =>
                            (name, size, sha256, chunk_size, chunk_count),
                        _ => return Err("应收到 FileMeta".into()),
                    };
                    let hash16: String = s.chars().take(16).collect();
                    // 查清单：匹配 → 已收块；不匹配 → 空（全量）
                    let done = match chunked::load_manifest(&hash16) {
                        Some(m) if m.file_size == size && m.chunk_size == chunk_size => m.done,
                        _ => Vec::new(),
                    };
                    assert_eq!(done, vec![0, 1], "清单应命中前两块");
                    let st = UdpMsg::ChunkStatus { done: done.clone() };
                    let st_json = serde_json::to_vec(&st).map_err(|e| format!("ChunkStatus 序列化: {e}"))?;
                    status_tx.send(st_json).map_err(|e| format!("status 转发: {e}"))?;
                    let p = rcv_progress.clone();
                    let (chunks, bytes) = quic_recv_chunks(&conn, &hash16, &name, size,
                        chunk_size, chunk_count, &done,
                        move |d, t| { p.lock().unwrap().push((d, t)); }).await?;
                    assert_eq!(chunks, 4, "应收满 4 块（含清单中的 2 块）");
                    assert_eq!(bytes, 8 * 1024 * 1024, "本次连接应只收到 8MB 新块");
                    let target = finalize_partial(&hash16, &name, &s, &Some(output_str.clone()))?;
                    assert_eq!(target, output, "应输出到 --output 指定路径");
                    conn.close(0u32.into(), b"done");
                    drop(ep);
                    Ok::<(), String>(())
                },
                async {
                    async_io::Timer::after(Duration::from_secs(30)).await;
                    Err("client2 30s 超时".to_string())
                },
            );
            let (srv_res, cli_res) = futures_lite::future::zip(srv, cli).await;
            srv_res?;
            cli_res
        });

        // ---------- 断言：续传只传缺失块 + 最终文件完整 ----------
        assert!(result.is_ok(), "续传应成功: {result:?}");
        assert_eq!(sha256_file(&output).unwrap(), sha, "续传结果应与原文件一致（文件级 SHA-256）");
        assert!(chunked::load_manifest(&hash16).is_none(), "收尾后清单应清理");
        assert!(!chunked::partial_path(&hash16).exists(), "收尾后 partial 应移除");
        let snd_last = snd_progress.lock().unwrap().last().copied();
        assert_eq!(snd_last, Some((8 * 1024 * 1024, 8 * 1024 * 1024)),
            "发送方进度应止于 8MB（只传缺失块）: {:?}", snd_progress.lock().unwrap());
        let rcv_last = rcv_progress.lock().unwrap().last().copied();
        assert_eq!(rcv_last, Some((16 * 1024 * 1024, 16 * 1024 * 1024)),
            "接收方进度应含清单中已收的 8MB: {:?}", rcv_progress.lock().unwrap());

        // 清理
        chunked::clear_manifest(&hash16);
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&output);
    }
}
