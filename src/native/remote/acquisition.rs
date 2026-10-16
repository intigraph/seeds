//! Explicit library-only acquisition. Ordinary reads and remote export do not use it.
//!
//! A trusted caller supplies the source identity and verifier manifest. This is
//! integrity verification, not producer authentication or production eligibility.
use super::{quipu_read_request, RemoteBackend};
use crate::error::{ErrorKind, Result, SdError};
use crate::pendant::sha256;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CONTRACT: &str = "quipu-acquisition/composed-v1";
const PROTOCOL: &str = "seeds-acquisition-verifier/v1";
const MAX_BYTES: usize = 268_435_456;
const CANONICALIZATION: &str = "source-labels-sorted-ntriples/v1";

/// Exact MAIN graph scope; comments live in the project graph.
#[derive(Debug, Clone, Serialize)]
pub struct AcquisitionRequest {
    pub source_store_id: String,
    pub graphs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_generation: Option<String>,
    pub max_bytes: usize,
}

/// An out-of-band trusted build manifest, never learned from a response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierManifest {
    pub executable: PathBuf,
    pub executable_sha256: String,
    pub verifier_source: String,
    pub verifier_lock_sha256: String,
    pub sdk_source: String,
    pub consumer_executable_sha256: String,
    pub sdk_lock_sha256: String,
    pub producer_source: String,
}

/// Only successful authoritative verification can construct this value.
pub struct VerifiedAcquisition {
    request: Vec<u8>,
    wire: Vec<u8>,
    manifest: VerifierManifest,
    version_sha256: String,
}

fn invalid(message: &str) -> SdError {
    SdError::refused(format!("acquisition UNKNOWN: {message}; no fallback"))
}

fn unreachable(message: &str) -> SdError {
    SdError::new(
        ErrorKind::Unreachable,
        format!("acquisition UNKNOWN: {message}; no fallback"),
    )
}

fn http_error(error: ureq::Error) -> SdError {
    match error {
        ureq::Error::Transport(_) => unreachable("transport failed"),
        ureq::Error::Status(code, _) if code == 408 || code == 429 || code >= 500 => {
            unreachable("server unavailable or read deadline")
        }
        ureq::Error::Status(_, _) => invalid("server explicitly refused contract/request"),
    }
}

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| unreachable("total deadline exceeded"))
}

fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl AcquisitionRequest {
    fn validate(&self, project: &str) -> Result<()> {
        if self.graphs.is_empty()
            || self.graphs.len() > 2
            || self.graphs.iter().any(|g| g.len() > 2048)
        {
            return Err(invalid("graph selection bounds"));
        }
        let ephemeral = format!("{project}#seeds-ephemeral");
        let mut scope = self.graphs.clone();
        scope.sort();
        scope.dedup();
        if self.source_store_id.is_empty()
            || self.source_store_id.len() > 256
            || self
                .expected_generation
                .as_ref()
                .is_some_and(|v| v.len() > 256)
            || self.max_bytes == 0
            || self.max_bytes > MAX_BYTES
            || self.tx.is_some_and(|tx| tx <= 0)
            || self.tx.is_some()
                && self
                    .expected_generation
                    .as_deref()
                    .is_none_or(str::is_empty)
            || self.expected_generation.as_deref() == Some("")
            || scope.len() != self.graphs.len()
            || !scope.iter().any(|g| g == project)
            || scope.iter().any(|g| g != project && g != &ephemeral)
            || scope.iter().any(|g| g.len() > 2048)
            || scope.iter().any(|g| oxiri::Iri::parse(g.as_str()).is_err())
        {
            return Err(invalid("request identity, pin, scope or lower-only budget"));
        }
        Ok(())
    }
}

/// Files are created exclusively inside a caller-owned scratch parent.
/// Drop removes only this invocation's directory, never caller input.
struct Scratch(PathBuf);
impl Scratch {
    fn create(parent: &Path) -> Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::getrandom(&mut random).map_err(|_| invalid("scratch entropy unavailable"))?;
        let name = sha256(&random).replace(':', "-");
        let dir = parent.join(format!("acquire-{name}"));
        fs::create_dir(&dir).map_err(|_| invalid("exclusive scratch creation failed"))?;
        let scratch = Self(dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o700))
                .map_err(|_| invalid("private scratch permissions"))?;
        }
        Ok(scratch)
    }
    fn file(&self, name: &str) -> Result<File> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(self.0.join(name))
            .map_err(|_| invalid("exclusive scratch file creation failed"))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bounded_read(mut reader: impl Read, cap: usize, deadline: Instant) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 16_384];
    loop {
        remaining(deadline)?;
        let n = reader
            .read(&mut buffer)
            .map_err(|_| unreachable("response read failed"))?;
        if n == 0 {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(n) > cap {
            return Err(invalid("byte ceiling exceeded"));
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
}

impl RemoteBackend {
    /// Acquire only when explicitly called by a separately authorized consumer.
    /// No CLI route calls this method. No retry or present-state fallback exists.
    pub fn acquire_verified(
        &self,
        request: &AcquisitionRequest,
        manifest: &VerifierManifest,
        scratch_parent: &Path,
        timeout: Duration,
    ) -> Result<VerifiedAcquisition> {
        if !cfg!(target_os = "linux") {
            return Err(SdError::new(
                ErrorKind::NotBuilt,
                "bounded acquisition verifier requires Linux; request not sent",
            ));
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| invalid("deadline"))?;
        if timeout.is_zero() || timeout > Duration::from_secs(120) {
            return Err(invalid("total timeout must be in (0,120s]"));
        }
        request.validate(&self.graph)?;
        if !manifest.executable.is_absolute()
            || !hex(&manifest.verifier_source, 40)
            || !hex(&manifest.sdk_source, 40)
            || !hex(&manifest.producer_source, 40)
            || !manifest.verifier_lock_sha256.starts_with("sha256:")
            || !hex(&manifest.verifier_lock_sha256[7..], 64)
            || !manifest.sdk_lock_sha256.starts_with("sha256:")
            || !hex(&manifest.sdk_lock_sha256[7..], 64)
        {
            return Err(invalid("trusted build manifest incomplete"));
        }
        if sha256(include_bytes!("../../../Cargo.lock")) != manifest.sdk_lock_sha256 {
            return Err(invalid("compiled SDK lock differs from manifest"));
        }
        let consumer =
            std::env::current_exe().map_err(|_| invalid("consumer executable unknown"))?;
        let consumer = bounded_read(
            File::open(consumer).map_err(|_| invalid("consumer executable unreadable"))?,
            MAX_BYTES,
            deadline,
        )?;
        if sha256(&consumer) != manifest.consumer_executable_sha256 {
            return Err(invalid("consumer executable digest mismatch"));
        }
        drop(consumer);
        let executable = bounded_read(
            File::open(&manifest.executable).map_err(|_| invalid("verifier unavailable"))?,
            MAX_BYTES,
            deadline,
        )?;
        if sha256(&executable) != manifest.executable_sha256 {
            return Err(invalid("verifier executable digest mismatch"));
        }
        // Execute a private copy of the exact hashed bytes, not a mutable PATH
        // or manifest pathname re-opened after the hash check.
        let scratch = Scratch::create(scratch_parent)?;
        let mut pinned = scratch.file("verifier")?;
        pinned
            .write_all(&executable)
            .and_then(|_| pinned.sync_all())
            .map_err(|_| invalid("private verifier staging"))?;
        drop(pinned);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                scratch.0.join("verifier"),
                fs::Permissions::from_mode(0o500),
            )
            .map_err(|_| invalid("private verifier permissions"))?;
        }
        drop(executable);
        let version = quipu_read_request(
            self.agent.get(&format!("{}/version", self.base)),
            "acquisition-version",
        )
        .timeout(remaining(deadline)?)
        .call()
        .map_err(http_error)?;
        let version = bounded_read(version.into_reader(), 65_536, deadline)?;
        let advertised: Value =
            serde_json::from_slice(&version).map_err(|_| invalid("version JSON"))?;
        if !advertised["read_contracts"]
            .as_array()
            .is_some_and(|v| v.iter().any(|v| v == CONTRACT))
        {
            return Err(invalid("composed contract not advertised"));
        }
        let request_bytes = serde_json::to_vec(request).map_err(|_| invalid("request encoding"))?;
        let response = quipu_read_request(
            self.agent.post(&format!("{}/acquire/composed", self.base)),
            "acquisition",
        )
        .timeout(remaining(deadline)?)
        .set("Content-Type", "application/json")
        .send_bytes(&request_bytes)
        .map_err(http_error)?;
        // JSON escaping and receipt metadata count against the caller's wire budget too.
        let wire = bounded_read(response.into_reader(), request.max_bytes, deadline)?;
        let mut input = scratch.file("input.json")?;
        let header = serde_json::to_vec(&json!({
            "schema":"seeds-acquisition-input/v1", "request_bytes":request_bytes.len(), "wire_bytes":wire.len()
        })).map_err(|_| invalid("verifier frame encoding"))?;
        input
            .write_all(&header)
            .and_then(|_| input.write_all(b"\n"))
            .and_then(|_| input.write_all(&request_bytes))
            .and_then(|_| input.write_all(&wire))
            .map_err(|_| invalid("verifier input persistence"))?;
        drop(input);
        let output = scratch.file("verdict.json")?;
        remaining(deadline)?;
        let mut child = Command::new(scratch.0.join("verifier"))
            .env_clear()
            .stdin(File::open(scratch.0.join("input.json")).map_err(|_| invalid("verifier input"))?)
            .stdout(Stdio::from(output))
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| invalid("verifier spawn failed"))?;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if remaining(deadline).is_ok() => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(invalid("verifier deadline or wait failure"));
                }
            }
        };
        if !status.success() {
            return Err(invalid("authoritative verifier refused"));
        }
        let verdict = bounded_read(
            File::open(scratch.0.join("verdict.json")).map_err(|_| invalid("verdict missing"))?,
            4096,
            deadline,
        )?;
        let verdict: Value =
            serde_json::from_slice(&verdict).map_err(|_| invalid("verdict malformed"))?;
        if verdict["schema"] != PROTOCOL
            || verdict["wire_sha256"] != sha256(&wire)
            || verdict["request_sha256"] != sha256(&request_bytes)
            || verdict["verifier_source"] != manifest.verifier_source
            || verdict["verifier_lock_sha256"] != manifest.verifier_lock_sha256
        {
            return Err(invalid("verdict binding mismatch"));
        }
        let payload: Value = serde_json::from_slice(&wire).map_err(|_| invalid("payload JSON"))?;
        let main = &payload["source_receipt"]["main"];
        if main["producer_git_sha"] != manifest.producer_source
            || main["producer_git_dirty"] != false
            || main["canonicalization"] != CANONICALIZATION
        {
            return Err(invalid("producer build/canonicalization mismatch"));
        }
        remaining(deadline)?;
        Ok(VerifiedAcquisition {
            request: request_bytes,
            wire,
            manifest: manifest.clone(),
            version_sha256: sha256(&version),
        })
    }
}

impl VerifiedAcquisition {
    /// Preserve original source bytes inside ONE atomic, create-only artifact.
    /// This local wrapper never substitutes for the source receipt it contains.
    /// Hard linking requires the scratch and destination to share a filesystem.
    pub fn persist(&self, destination: &Path, timeout: Duration) -> Result<()> {
        if timeout.is_zero() || timeout > Duration::from_secs(120) {
            return Err(invalid("persistence deadline"));
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| invalid("deadline"))?;
        let parent = destination
            .parent()
            .ok_or_else(|| invalid("artifact parent missing"))?;
        let scratch = Scratch::create(parent)?;
        let mut file = scratch.file("artifact.acq")?;
        let receipt = serde_json::to_vec(&json!({
            "schema":"seeds-acquisition-artifact/v1",
            "request_bytes":self.request.len(), "wire_bytes":self.wire.len(),
            "wire_sha256":sha256(&self.wire), "request_sha256":sha256(&self.request),
            "version_sha256":self.version_sha256, "verifier":self.manifest
        }))
        .map_err(|_| invalid("local receipt encoding"))?;
        file.write_all(&receipt)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.write_all(&self.request))
            .and_then(|_| file.write_all(&self.wire))
            .and_then(|_| file.sync_all())
            .map_err(|_| invalid("artifact staging failed"))?;
        remaining(deadline)?;
        fs::hard_link(scratch.0.join("artifact.acq"), destination)
            .map_err(|_| invalid("artifact publish refused; destination must not exist"))?;
        // From this point a durability error is indeterminate, never safe to replay.
        File::open(parent).and_then(|f| f.sync_all()).map_err(|_| {
            SdError::new(
                ErrorKind::Indeterminate,
                "artifact linked but parent durability unproven; read before retry",
            )
        })?;
        if remaining(deadline).is_err() {
            return Err(SdError::new(
                ErrorKind::Indeterminate,
                "artifact linked after deadline; read before retry",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AcquisitionRequest {
        AcquisitionRequest {
            source_store_id: "owned-lineage".into(),
            graphs: vec![
                "https://example.org/board".into(),
                "https://example.org/board#seeds-ephemeral".into(),
            ],
            tx: Some(7),
            expected_generation: Some("observed-generation".into()),
            max_bytes: 1024,
        }
    }

    #[test]
    fn exact_scope_and_pin_admission_before_io() {
        let r = request();
        assert!(r.validate("https://example.org/board").is_ok());
        let mut bad = r.clone();
        bad.graphs.push(bad.graphs[0].clone());
        assert!(bad.validate("https://example.org/board").is_err());
        let mut bad = r.clone();
        bad.graphs[1] = "https://example.org/comments".into();
        assert!(bad.validate("https://example.org/board").is_err());
        let mut bad = r.clone();
        bad.expected_generation = None;
        assert!(bad.validate("https://example.org/board").is_err());
        let mut bad = r.clone();
        bad.tx = Some(-1);
        assert!(bad.validate("https://example.org/board").is_err());
        let mut bad = r;
        bad.max_bytes = MAX_BYTES + 1;
        assert!(bad.validate("https://example.org/board").is_err());
        let mut present = request();
        present.tx = None;
        present.expected_generation = None;
        assert!(present.validate("https://example.org/board").is_ok());
        // The producer refuses explicit null pins; omit absent typed options.
        let wire = serde_json::to_value(present).unwrap();
        assert!(wire.get("tx").is_none());
        assert!(wire.get("expected_generation").is_none());
    }

    #[test]
    fn bounded_reader_refuses_overflow_and_expired_deadline() {
        let until = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            bounded_read(&b"positive"[..], 8, until).unwrap(),
            b"positive"
        );
        assert!(bounded_read(&b"positive"[..], 7, until).is_err());
        assert!(bounded_read(&b""[..], 1, Instant::now()).is_err());
    }

    #[test]
    fn unavailable_read_is_not_an_empty_or_definite_success() {
        let timeout = http_error(ureq::Error::Status(
            408,
            ureq::Response::new(408, "timeout", "").unwrap(),
        ));
        assert_eq!(timeout.kind, ErrorKind::Unreachable);
        let refused = http_error(ureq::Error::Status(
            422,
            ureq::Response::new(422, "refused", "").unwrap(),
        ));
        assert_eq!(refused.kind, ErrorKind::Refused);
    }

    #[test]
    fn artifact_is_create_only_and_preserves_source_bytes() {
        // Local persistence unit control, not an acquisition/producer trial.
        let parent = Scratch::create(&std::env::temp_dir()).unwrap();
        let wire =
            b" \n{ \"source_receipt\" : {\"identity\": \"untouched\"}, \"graphs\":{} }\t".to_vec();
        let a = VerifiedAcquisition {
            request: serde_json::to_vec(&request()).unwrap(),
            wire: wire.clone(),
            version_sha256: "local-unit-control".into(),
            manifest: VerifierManifest {
                executable: "/not/executed".into(),
                executable_sha256: "not-executed".into(),
                verifier_source: "not-executed".into(),
                verifier_lock_sha256: "unit-control".into(),
                sdk_source: "unit-control".into(),
                consumer_executable_sha256: "not-executed".into(),
                sdk_lock_sha256: "unit-control".into(),
                producer_source: "unit-control".into(),
            },
        };
        let destination = parent.0.join("artifact.acq");
        a.persist(&destination, Duration::from_secs(2)).unwrap();
        let before = fs::read(&destination).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let header_end = before.iter().position(|b| *b == b'\n').unwrap();
        let receipt: Value = serde_json::from_slice(&before[..header_end]).unwrap();
        assert_eq!(receipt["schema"], "seeds-acquisition-artifact/v1");
        assert_eq!(receipt["wire_sha256"], sha256(&wire));
        assert_eq!(receipt["request_sha256"], sha256(&a.request));
        assert_eq!(receipt["request_bytes"], a.request.len());
        assert_eq!(receipt["wire_bytes"], wire.len());
        let body = &before[header_end + 1..];
        assert_eq!(&body[..a.request.len()], a.request);
        assert_eq!(&body[a.request.len()..], wire);
        assert!(a.persist(&destination, Duration::from_secs(2)).is_err());
        assert_eq!(fs::read(&destination).unwrap(), before);
        assert_eq!(fs::read_dir(&parent.0).unwrap().count(), 1);
    }
}
