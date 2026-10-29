//! The certificate a server shows, asked for by Kova itself, so the greeting can be cut
//! the way Anti-DPI cuts it (Xray's "tlshello" fragment) instead of going out whole.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ring::rand::{SecureRandom, SystemRandom};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Takes any certificate and keeps the hex SHA-256 of the leaf, as Xray's
/// `pinnedPeerCertSha256` and `xray tls ping` write it.
#[derive(Debug)]
struct Recorder {
    leaf: Mutex<Option<String>>,
    schemes: Vec<SignatureScheme>,
}

impl ServerCertVerifier for Recorder {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let sum = ring::digest::digest(&ring::digest::SHA256, end_entity.as_ref());
        *self.leaf.lock().unwrap() = Some(hex::encode(sum.as_ref()));
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.schemes.clone()
    }
}

/// The leaf certificate's SHA-256 of `addr` asked for as `name`, or None when the server
/// doesn't finish showing it within `limit`. With `fragment` the greeting goes out as
/// TLS records of 100-200 bytes, 10-20 ms apart.
pub async fn leaf_sha256(addr: SocketAddr, name: &str, fragment: bool, limit: Duration) -> Option<String> {
    tokio::time::timeout(limit, probe(addr, name, fragment)).await.ok().flatten()
}

async fn probe(addr: SocketAddr, name: &str, fragment: bool) -> Option<String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let recorder = Arc::new(Recorder { leaf: Mutex::new(None), schemes: provider.signature_verification_algorithms.supported_schemes() });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .ok()?
        .dangerous()
        .with_custom_certificate_verifier(recorder.clone())
        .with_no_client_auth();
    let server_name = ServerName::try_from(name.to_string()).ok()?;
    let mut conn = ClientConnection::new(Arc::new(config), server_name).ok()?;

    let mut tcp = TcpStream::connect(addr).await.ok()?;
    tcp.set_nodelay(true).ok()?;
    let mut first = fragment;
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        while conn.wants_write() {
            let mut out = Vec::new();
            conn.write_tls(&mut out).ok()?;
            if std::mem::take(&mut first) {
                send_cut(&mut tcp, &out).await?;
            } else {
                tcp.write_all(&out).await.ok()?;
            }
        }
        if let Some(pin) = recorder.leaf.lock().unwrap().take() {
            return Some(pin);
        }
        if !conn.is_handshaking() {
            return None;
        }
        let n = tcp.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        conn.read_tls(&mut &buf[..n]).ok()?;
        // A failure after the certificate was seen still leaves the pin.
        let processed = conn.process_new_packets();
        if let Some(pin) = recorder.leaf.lock().unwrap().take() {
            return Some(pin);
        }
        processed.ok()?;
    }
}

/// Write the first handshake record as several smaller records, each with its own header.
async fn send_cut(tcp: &mut TcpStream, out: &[u8]) -> Option<()> {
    if out.len() < 5 || out[0] != 0x16 {
        return tcp.write_all(out).await.ok();
    }
    let len = usize::from(u16::from_be_bytes([out[3], out[4]]));
    let body = out.get(5..5 + len)?;
    let rng = SystemRandom::new();
    let pick = |lo: u64, hi: u64| -> u64 {
        let mut b = [0u8; 2];
        let _ = rng.fill(&mut b);
        lo + u64::from(u16::from_be_bytes(b)) % (hi - lo + 1)
    };
    let mut rest = body;
    while !rest.is_empty() {
        let take = (pick(100, 200) as usize).min(rest.len());
        let (piece, tail) = rest.split_at(take);
        let mut record = Vec::with_capacity(5 + piece.len());
        record.extend_from_slice(&out[..3]);
        record.extend_from_slice(&(piece.len() as u16).to_be_bytes());
        record.extend_from_slice(piece);
        tcp.write_all(&record).await.ok()?;
        rest = tail;
        if !rest.is_empty() {
            tokio::time::sleep(Duration::from_millis(pick(10, 20))).await;
        }
    }
    // Whatever followed the greeting in the same flight.
    tcp.write_all(&out[5 + len..]).await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pieces put back together are the record that was cut, and each stays small.
    #[tokio::test]
    async fn the_greeting_is_cut_into_small_records() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body: Vec<u8> = (0..700u32).map(|i| i as u8).collect();
        let mut record = vec![0x16, 0x03, 0x01];
        record.extend_from_slice(&(body.len() as u16).to_be_bytes());
        record.extend_from_slice(&body);
        let sent = record.clone();
        let writer = tokio::spawn(async move {
            let mut tcp = TcpStream::connect(addr).await.unwrap();
            send_cut(&mut tcp, &sent).await.unwrap();
        });
        let (mut peer, _) = listener.accept().await.unwrap();
        writer.await.unwrap();
        let mut got = Vec::new();
        peer.read_to_end(&mut got).await.unwrap();
        let mut joined = Vec::new();
        let mut at = 0;
        let mut pieces = 0;
        while at < got.len() {
            assert_eq!(&got[at..at + 3], &[0x16, 0x03, 0x01]);
            let len = usize::from(u16::from_be_bytes([got[at + 3], got[at + 4]]));
            assert!(len <= 200);
            joined.extend_from_slice(&got[at + 5..at + 5 + len]);
            at += 5 + len;
            pieces += 1;
        }
        assert_eq!(joined, body);
        assert!(pieces >= 4);
    }
}
