//! Custom roots replace system roots; no insecure verifier or fallback.
use crate::error::{Result, SdError};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc, time::Duration};
use ureq::rustls::{
    self,
    pki_types::{pem::PemObject, CertificateDer},
};

pub(super) fn agent(ca_file: Option<&Path>, pin: Option<&str>) -> Result<ureq::Agent> {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(120));
    if let Some(path) = ca_file {
        let refuse = |why: &str| SdError::refused(format!("CA file {}: {why}", path.display()));
        let bytes =
            std::fs::read(path).map_err(|_| refuse("cannot read configured PEM trust roots"))?;
        if let Some(expected) = pin {
            if format!("{:x}", Sha256::digest(&bytes)) != expected {
                return Err(refuse("SHA-256 mismatch"));
            }
        }
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(&bytes) {
            roots
                .add(cert.map_err(|_| refuse("invalid PEM certificate"))?)
                .map_err(|_| refuse("invalid CA certificate"))?;
        }
        if roots.is_empty() {
            return Err(refuse("no CA certificates in PEM file"));
        }
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        builder = builder.tls_config(Arc::new(config));
    } else if pin.is_some() {
        return Err(SdError::refused("ca_sha256 requires ca_file"));
    }
    Ok(builder.build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    use ureq::rustls::pki_types::PrivateKeyDer;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/tls/generate.py"
            ))
            .arg(dir.path())
            .status()
            .unwrap()
            .success());
        dir
    }

    fn server(host: &str, dir: &Path) -> (String, thread::JoinHandle<()>) {
        let cert_pem = std::fs::read(dir.join("server.pem")).unwrap();
        let certs = CertificateDer::pem_slice_iter(&cert_pem)
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        let key =
            PrivateKeyDer::from_pem_slice(&std::fs::read(dir.join("server.key")).unwrap()).unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("https://{host}:{}", listener.local_addr().unwrap().port());
        let worker = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let conn = rustls::ServerConnection::new(Arc::new(config)).unwrap();
            let mut stream = rustls::StreamOwned::new(conn, socket);
            let mut buf = [0; 4096];
            if stream.read(&mut buf).is_ok() {
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                );
                let _ = stream.flush();
            }
        });
        (url, worker)
    }

    #[test]
    fn ca_verifies_but_foreign_ca_default_roots_and_wrong_hostname_refuse() {
        let dir = fixture();
        let path = dir.path().join("ca.pem");
        let ca = std::fs::read(&path).unwrap();
        let pin = format!("{:x}", Sha256::digest(&ca));
        let (url, worker) = server("localhost", dir.path());
        assert_eq!(
            agent(Some(&path), Some(&pin))
                .unwrap()
                .get(&url)
                .call()
                .unwrap()
                .into_string()
                .unwrap(),
            "OK"
        );
        worker.join().unwrap();
        for (roots, host) in [(Some(path.as_path()), "127.0.0.1"), (None, "localhost")] {
            let (url, worker) = server(host, dir.path());
            assert!(agent(roots, None).unwrap().get(&url).call().is_err());
            worker.join().unwrap();
        }
        std::fs::write(
            &path,
            std::fs::read(dir.path().join("foreign-ca.pem")).unwrap(),
        )
        .unwrap();
        let (url, worker) = server("localhost", dir.path());
        assert!(agent(Some(&path), None).unwrap().get(&url).call().is_err());
        worker.join().unwrap();
    }

    #[test]
    fn missing_invalid_and_digest_mismatched_ca_never_fall_back() {
        let dir = fixture();
        let ca = std::fs::read(dir.path().join("ca.pem")).unwrap();
        let path = dir.path().join("configured-ca.pem");
        let error = agent(Some(&path), None).unwrap_err().to_string();
        assert!(error.contains(path.to_str().unwrap()));
        for data in [
            b"".as_slice(),
            b"invalid PEM",
            b"-----BEGIN CERTIFICATE-----\ninvalid!\n-----END CERTIFICATE-----",
        ] {
            std::fs::write(&path, data).unwrap();
            assert!(agent(Some(&path), None).is_err());
        }
        std::fs::write(&path, &ca).unwrap();
        assert!(agent(Some(&path), Some(&"0".repeat(64))).is_err());
        assert!(agent(None, Some(&"0".repeat(64))).is_err());
    }
}
