//! QUIC 直连：quinn 在打通的 UDP socket 上建连接，证书指纹经 Wormhole 通道交换。

use std::net::UdpSocket;
use std::sync::Arc;
use std::time::Duration;

/// QUIC 强制要求 ALPN 协商一致，两端用同一个应用层协议名（本工具自定，不对外兼容）。
const QUIC_ALPN: &[u8] = b"fan-files/1";
/// 自签证书 SAN 域名与客户端 SNI server name（两端一致；QUIC 要求 DNS name 形式）。
const QUIC_SERVER_NAME: &str = "fan-files.local";

/// 证书 SHA-256 指纹（hex）——经 Wormhole 通道预交换，握手时固定校验（防 MITM）。
pub fn cert_fingerprint(cert: &rustls_pki_types::CertificateDer) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(cert.as_ref());
    d.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 生成自签证书（打洞 QUIC 用；指纹经 Wormhole 通道验证，无需 CA）
pub fn self_signed_cert() -> (rustls_pki_types::CertificateDer<'static>, rustls_pki_types::PrivateKeyDer<'static>) {
    // 用 rcgen 生成自签 ECDSA 证书（域名 QUIC_SERVER_NAME，QUIC 需要 SAN）
    let key_pair = rcgen::KeyPair::generate().expect("keypair gen");
    let cert = rcgen::CertificateParams::new(vec![QUIC_SERVER_NAME.to_string()])
        .expect("cert params")
        .self_signed(&key_pair)
        .expect("self-signed cert");
    (cert.into(), key_pair.into())
}

/// 生成后打印/返回指纹（调用方经 Wormhole 通道交换）
pub fn gen_cert_with_fingerprint() -> (rustls_pki_types::CertificateDer<'static>, rustls_pki_types::PrivateKeyDer<'static>, String) {
    let (c, k) = self_signed_cert();
    let fp = cert_fingerprint(&c);
    (c, k, fp)
}

/// 常量时间指纹比较：XOR 累加逐字节比较 + 长度守卫，避免字符串 `==` 在首字节不同时
/// 提前退出带来的时序泄露（长度泄露可接受：hex 指纹是公开的固定长度格式）。
fn fp_matches(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.as_bytes().iter().zip(b.as_bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 指纹固定 verifier：仅信任指纹匹配的自签证书（防 MITM 核心）。
/// quinn 默认 WebPkiVerifier 会拒绝自签证书，必须用这个自定义 verifier。
#[derive(Debug)]
pub struct FingerprintVerifier {
    /// 经 Wormhole 通道交换的对端证书指纹（SHA-256 hex）
    pub expected: String,
}

impl rustls::client::danger::ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls_pki_types::CertificateDer,
        _intermediates: &[rustls_pki_types::CertificateDer],
        _server_name: &rustls_pki_types::ServerName,
        _ocsp: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let fp = cert_fingerprint(end_entity);
        if fp_matches(&fp, &self.expected) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "fingerprint mismatch: expected {} got {}",
                self.expected, fp
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls_pki_types::CertificateDer,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls_pki_types::CertificateDer,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// 服务端：在打通的 UDP socket 上 accept QUIC 连接（证书自签，指纹由客户端校验）。
/// sock 需已绑定且非阻塞；调用方持有返回的 Endpoint（drop 会关闭所有连接）。
///
/// **重要（T5 集成必读）**：quinn 只有在对入站连接调用 accept() 并 await 后才会推进
/// 握手。调用方须在 quic_listen 返回后立即 `endpoint.accept().await` 拿到 Incoming 再
/// `.await`（内部接受并驱动握手，得到服务端 Connection）。若一直不 accept，服务端不会
/// 响应客户端，客户端会反复重传 Initial 直到握手超时。
pub async fn quic_listen(
    sock: UdpSocket,
    cert: rustls_pki_types::CertificateDer<'static>,
    key: rustls_pki_types::PrivateKeyDer<'static>,
) -> Result<quinn::Endpoint, String> {
    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|e| format!("tls versions: {e}"))?
    .with_no_client_auth()
    .with_single_cert(vec![cert], key)
    .map_err(|e| format!("server cert: {e}"))?;
    // QUIC 强制 ALPN 协商：与客户端（quic_connect）必须一致，否则握手失败
    tls.alpn_protocols = vec![QUIC_ALPN.to_vec()];
    // QUIC 要求 max_early_data_size 为 0 或 u32::MAX；0 = 关闭 0-RTT（客户端未开 0-RTT，保守）
    tls.max_early_data_size = 0;
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(Arc::new(tls))
            .map_err(|e| format!("quic server config: {e}"))?,
    ));
    let runtime = quinn::default_runtime().ok_or("quinn runtime unavailable")?;
    quinn::Endpoint::new(quinn::EndpointConfig::default(), Some(server_config), sock, runtime)
        .map_err(|e| format!("server endpoint: {e}"))
}

/// 客户端：用期望指纹建连（校验对端证书，防 MITM）。
/// 返回 `(endpoint, conn)`：**endpoint 必须由调用方持有**——drop endpoint 会立刻关闭所有连接。
/// `timeout` 为握手超时（quinn 自身的握手超时默认 30s，T5 建议传 10s），超时返回 Err。
pub async fn quic_connect(
    sock: UdpSocket,
    peer: std::net::SocketAddr,
    expected_fp: String,
    timeout: Duration,
) -> Result<(quinn::Endpoint, quinn::Connection), String> {
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|e| format!("tls versions: {e}"))?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(FingerprintVerifier { expected: expected_fp }))
    .with_no_client_auth();
    tls.alpn_protocols = vec![QUIC_ALPN.to_vec()];
    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(Arc::new(tls))
            .map_err(|e| format!("quic client config: {e}"))?,
    ));
    let runtime = quinn::default_runtime().ok_or("quinn runtime unavailable")?;
    let endpoint = quinn::Endpoint::new(quinn::EndpointConfig::default(), None, sock, runtime)
        .map_err(|e| format!("client endpoint: {e}"))?;
    // Connecting 在块外创建：endpoint 必须留在外层由调用方持有（进入块内会被提前 drop
    // 而立刻关闭所有连接），块内只 await 握手。
    let connecting = endpoint
        .connect_with(client_config, peer, QUIC_SERVER_NAME)
        .map_err(|e| format!("connect: {e}"))?;
    let conn = futures_lite::future::or(
        async { connecting.await.map_err(|e| format!("handshake: {e}")) },
        async {
            async_io::Timer::after(timeout).await;
            Err(format!("handshake timeout after {}ms", timeout.as_millis()))
        },
    )
    .await?;
    Ok((endpoint, conn))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::client::danger::ServerCertVerifier;
    use std::time::Duration;

    fn server_name() -> rustls_pki_types::ServerName<'static> {
        "fan-files.local".try_into().unwrap()
    }

    /// 翻转指纹第一个 hex 字符 → 必然与真实指纹不同
    fn wrong_fp(fp: &str) -> String {
        let mut b = fp.as_bytes().to_vec();
        b[0] = if b[0] == b'0' { b'1' } else { b'0' };
        String::from_utf8(b).unwrap()
    }

    fn bind_udp() -> (std::net::UdpSocket, std::net::SocketAddr) {
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_nonblocking(true).unwrap();
        let addr = sock.local_addr().unwrap();
        (sock, addr)
    }

    #[test]
    fn cert_fingerprint_is_deterministic() {
        let cert = rustls_pki_types::CertificateDer::from(vec![1u8, 2, 3]);
        let a = cert_fingerprint(&cert);
        let b = cert_fingerprint(&cert);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // SHA-256 hex
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn different_certs_differ() {
        let c1 = rustls_pki_types::CertificateDer::from(vec![1u8, 2, 3]);
        let c2 = rustls_pki_types::CertificateDer::from(vec![1u8, 2, 4]);
        assert_ne!(cert_fingerprint(&c1), cert_fingerprint(&c2));
    }

    // ---------- 指纹固定 verifier 单元测试（核心安全逻辑，无网络） ----------

    #[test]
    fn verifier_rejects_wrong_fingerprint() {
        let (cert, _key, fp) = gen_cert_with_fingerprint();
        let verifier = FingerprintVerifier { expected: wrong_fp(&fp) };
        let result = verifier.verify_server_cert(
            &cert,
            &[],
            &server_name(),
            &[],
            rustls_pki_types::UnixTime::now(),
        );
        assert!(result.is_err(), "指纹不匹配必须被拒绝");
        let err = format!("{:?}", result.unwrap_err());
        assert!(err.contains("fingerprint mismatch"), "错误信息应指出指纹不匹配: {err}");
    }

    #[test]
    fn verifier_accepts_matching_fingerprint() {
        let (cert, _key, fp) = gen_cert_with_fingerprint();
        let verifier = FingerprintVerifier { expected: fp };
        let result = verifier.verify_server_cert(
            &cert,
            &[],
            &server_name(),
            &[],
            rustls_pki_types::UnixTime::now(),
        );
        assert!(result.is_ok(), "指纹匹配应通过: {:?}", result.err());
    }

    // ---------- 本地握手集成测试（同机双 endpoint，真实 QUIC 握手） ----------
    //
    // 注意：quinn 的入站连接只有在服务端 accept 并驱动后才会推进握手（否则服务端
    // 不响应，客户端会一直重传 Initial 直至超时），所以测试里服务端必须 accept。
    // 运行在 async_std::task::block_on 下：quinn 的 runtime-async-std 驱动任务
    // 跑在 async-std executor 上，需要它来推进。async_io::block_on 也可用（见
    // handshake_succeeds 双 executor 断言）。

    /// 正确指纹握手：服务端 accept 并驱动入站连接，客户端握手成功后检查连接存活。
    /// 返回连接在 quic_connect 返回后是否仍然存活（endpoint 被提前 drop 则死亡）。
    async fn correct_fp_handshake_probe() -> Result<bool, String> {
        let (cert, key, fp) = gen_cert_with_fingerprint();
        let (server_sock, addr) = bind_udp();
        let (client_sock, _) = bind_udp();

        let server = quic_listen(server_sock, cert, key).await.expect("server endpoint");
        let srv = futures_lite::future::or(
            async {
                let incoming = server.accept().await.ok_or("server accept 超时")?;
                let conn = incoming
                    .await
                    .map_err(|e| format!("incoming handshake: {e}"))?;
                let _ = conn.closed().await;
                Ok::<(), String>(())
            },
            async {
                async_io::Timer::after(Duration::from_secs(10)).await;
                Err("server accept 10s 超时".to_string())
            },
        );
        let cli = async {
            // 握手超时已内置在 quic_connect 的 timeout 参数里（10s）
            let (endpoint, conn) = quic_connect(client_sock, addr, fp, Duration::from_secs(10)).await?;
            // 连接必须在 quic_connect 返回后仍存活（endpoint 被提前 drop 会立刻关闭）
            let alive = futures_lite::future::or(
                async {
                    conn.closed().await;
                    false
                },
                async {
                    async_io::Timer::after(Duration::from_millis(300)).await;
                    true
                },
            )
            .await;
            conn.close(0u32.into(), b"test done");
            drop(endpoint);
            Ok::<bool, String>(alive)
        };
        let (srv_res, cli_res) = futures_lite::future::zip(srv, cli).await;
        srv_res?;
        cli_res
    }

    #[test]
    fn handshake_succeeds_with_correct_fingerprint() {
        // quinn 的 runtime-async-std 驱动任务在 async-std executor 上；两种 block_on 都应可用
        let alive = async_std::task::block_on(correct_fp_handshake_probe())
            .expect("async_std::task::block_on 下握手应成功");
        assert!(alive, "连接在 quic_connect 返回后立刻死亡（endpoint 被提前 drop?）");

        let alive = async_io::block_on(correct_fp_handshake_probe())
            .expect("async_io::block_on 下握手也应成功");
        assert!(alive, "async_io::block_on 下连接应同样存活");
    }

    /// 错误指纹握手：服务端 accept 并驱动入站连接，客户端必须快速失败（非超时）。
    async fn wrong_fp_handshake_probe() -> Result<(), String> {
        let (cert, key, fp) = gen_cert_with_fingerprint();
        let (server_sock, addr) = bind_udp();
        let (client_sock, _) = bind_udp();

        let server = quic_listen(server_sock, cert, key).await.expect("server endpoint");
        let srv = futures_lite::future::or(
            async {
                // 客户端校验失败会中止握手，服务端这里容忍错误
                if let Some(incoming) = server.accept().await {
                    let _ = incoming.await;
                }
            },
            async {
                async_io::Timer::after(Duration::from_secs(10)).await;
            },
        );
        let cli = async {
            // 错误指纹应在握手阶段快速失败；握手超时内置在 quic_connect（10s）
            quic_connect(client_sock, addr, wrong_fp(&fp), Duration::from_secs(10))
                .await
                .map(|_| ())
        };
        let (_srv, cli_res) = futures_lite::future::zip(srv, cli).await;
        cli_res
    }

    #[test]
    fn handshake_rejects_wrong_fingerprint() {
        let err = async_std::task::block_on(wrong_fp_handshake_probe())
            .expect_err("错误指纹的握手必须失败");
        assert!(!err.contains("timeout"), "应是快速拒绝而不是超时: {err}");
        assert!(err.contains("handshake"), "应是握手阶段失败: {err}");
    }

    // ---------- T5 路径预验证：打洞→QUIC 无缝组合 + stream 数据回环 ----------

    /// 打洞打通的两个 UDP socket 上直接建 QUIC：punch_establish（loopback 必成功）
    /// 返回的 socket 立刻交给 quic_listen/quic_connect，握手应正常完成。
    #[test]
    fn punch_then_quic_handshake() {
        // 双端 loopback 打洞（同机必打通）
        let s1 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let s2 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let a1 = s1.local_addr().unwrap();
        let a2 = s2.local_addr().unwrap();
        drop(s1);
        drop(s2);
        let t1 = std::thread::spawn(move || {
            crate::commands::udp_punch::punch_establish(a1, a2, 7, "peer-a".into(), Duration::from_secs(2))
        });
        std::thread::sleep(Duration::from_millis(100)); // 冗余保险：错开首拍
        let t2 = std::thread::spawn(move || {
            crate::commands::udp_punch::punch_establish(a2, a1, 7, "peer-b".into(), Duration::from_secs(2))
        });
        let sock_a = t1.join().unwrap().expect("peer-a 打洞应成功");
        let sock_b = t2.join().unwrap().expect("peer-b 打洞应成功");
        // 注：socket 接收缓冲区里残留的 punch 补发包对 quinn 无害——解析为短头未知连接
        // 直接丢弃，无需 drain。

        let (cert, key, fp) = gen_cert_with_fingerprint();
        let result = async_std::task::block_on(async {
            let server = quic_listen(sock_a, cert, key).await.expect("server endpoint");
            let srv = futures_lite::future::or(
                async {
                    let incoming = server.accept().await.ok_or("server accept 超时")?;
                    let conn = incoming
                        .await
                        .map_err(|e| format!("incoming handshake: {e}"))?;
                    let _ = conn.closed().await;
                    Ok::<(), String>(())
                },
                async {
                    async_io::Timer::after(Duration::from_secs(10)).await;
                    Err("server accept 10s 超时".to_string())
                },
            );
            // 不 await：把 future 直接交给 zip 与 srv 并发驱动。客户端 sock_b（绑在 a2）
            // 连服务端 sock_a 的地址 a1；握手完成后主动 close，服务端 conn.closed() 才会返回。
            let cli = async {
                let (endpoint, conn) = quic_connect(sock_b, a1, fp, Duration::from_secs(10)).await?;
                conn.close(0u32.into(), b"punch-quic done");
                drop(endpoint);
                Ok::<(), String>(())
            };
            let (srv_res, cli_res) = futures_lite::future::zip(srv, cli).await;
            srv_res?;
            cli_res
        });
        assert!(result.is_ok(), "打洞打通后的 socket 上 QUIC 握手应成功: {result:?}");
    }

    /// 1 字节双向 stream 回环：客户端 open_bi 发送 0x2a，服务端 accept_bi 读出后
    /// 回写 0x2b，客户端再读出——验证 QUIC 建连后数据传输链路（T5 传输路径）。
    #[test]
    fn quic_stream_roundtrip_one_byte() {
        let (cert, key, fp) = gen_cert_with_fingerprint();
        let (server_sock, addr) = bind_udp();
        let (client_sock, _) = bind_udp();

        let result = async_std::task::block_on(async {
            let server = quic_listen(server_sock, cert, key).await.expect("server endpoint");
            let srv = futures_lite::future::or(
                async {
                    let incoming = server.accept().await.ok_or("server accept 超时")?;
                    let conn = incoming
                        .await
                        .map_err(|e| format!("incoming handshake: {e}"))?;
                    let (mut send, mut recv) =
                        conn.accept_bi().await.map_err(|e| format!("accept_bi: {e}"))?;
                    let mut buf = [0u8; 4];
                    let n = recv
                        .read(&mut buf)
                        .await
                        .map_err(|e| format!("recv: {e}"))?
                        .ok_or("对端 EOF")?;
                    assert_eq!(n, 1);
                    assert_eq!(buf[0], 0x2a, "服务端应收 0x2a");
                    send.write_all(&[0x2b]).await.map_err(|e| format!("send: {e}"))?;
                    send.finish().map_err(|e| format!("finish: {e}"))?;
                    let _ = conn.closed().await;
                    Ok::<(), String>(())
                },
                async {
                    async_io::Timer::after(Duration::from_secs(10)).await;
                    Err("server 侧 10s 超时".to_string())
                },
            );
            let cli = futures_lite::future::or(
                async {
                    let (endpoint, conn) =
                        quic_connect(client_sock, addr, fp, Duration::from_secs(10)).await?;
                    let (mut send, mut recv) =
                        conn.open_bi().await.map_err(|e| format!("open_bi: {e}"))?;
                    send.write_all(&[0x2a]).await.map_err(|e| format!("send: {e}"))?;
                    send.finish().map_err(|e| format!("finish: {e}"))?;
                    let mut buf = [0u8; 4];
                    let n = recv
                        .read(&mut buf)
                        .await
                        .map_err(|e| format!("recv: {e}"))?
                        .ok_or("对端 EOF")?;
                    assert_eq!(n, 1);
                    assert_eq!(buf[0], 0x2b, "客户端应收 0x2b");
                    conn.close(0u32.into(), b"test done");
                    drop(endpoint);
                    Ok::<(), String>(())
                },
                async {
                    async_io::Timer::after(Duration::from_secs(10)).await;
                    Err("client 侧 10s 超时".to_string())
                },
            );
            let (srv_res, cli_res) = futures_lite::future::zip(srv, cli).await;
            srv_res?;
            cli_res
        });
        assert!(result.is_ok(), "1 字节双向 stream 回环应成功: {result:?}");
    }
}
