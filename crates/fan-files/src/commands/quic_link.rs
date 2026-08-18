//! QUIC 直连：quinn 在打通的 UDP socket 上建连接，证书指纹经 Wormhole 通道交换。

/// 证书 SHA-256 指纹（hex）——经 Wormhole 通道预交换，握手时固定校验（防 MITM）。
pub fn cert_fingerprint(cert: &rustls_pki_types::CertificateDer) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(cert.as_ref());
    d.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 生成自签证书（打洞 QUIC 用；指纹经 Wormhole 通道验证，无需 CA）
pub fn self_signed_cert() -> (rustls_pki_types::CertificateDer<'static>, rustls_pki_types::PrivateKeyDer<'static>) {
    // 用 rcgen 生成自签 ECDSA 证书（域名 fan-files.local，QUIC 需要 SAN）
    let key_pair = rcgen::KeyPair::generate().expect("keypair gen");
    let cert = rcgen::CertificateParams::new(vec!["fan-files.local".to_string()])
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
