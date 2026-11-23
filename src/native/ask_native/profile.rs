//! Root-selected code and reader identity, never browser-supplied admission.
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Result, SdError};

pub(super) const ASSETS: [&str; 7] = [
    "scripts/verify-seeds-answer.mjs",
    "scripts/admission/verify-seeds-answer.mjs",
    "web/deck/value.mjs",
    "web/deck/model.mjs",
    "web/deck/evidence.mjs",
    "web/deck/reader.mjs",
    "web/deck/settlement.mjs",
];

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Artifact {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    enabled: bool,
    admitted: Option<Profile>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Profile {
    pub endpoint: String,
    pub graph: String,
    pub audit_location: String,
    pub store_id: String,
    pub machine_principal: String,
    pub custody_directory: PathBuf,
    pub custody_uid: u32,
    pub writer_credential_file: Option<PathBuf>,
    pub code_root: PathBuf,
    pub node: Artifact,
    pub assets: BTreeMap<String, String>,
    pub reader: Value,
    pub tls_ca: Option<Artifact>,
}

pub(super) fn refused() -> SdError {
    SdError::refused("native Ask settlement profile/code is unadmitted or incomplete")
}

pub(super) fn absolute(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|p| matches!(p, Component::ParentDir | Component::CurDir))
    {
        return Err(refused());
    }
    Ok(())
}

fn parents(path: &Path, owner: u32) -> Result<()> {
    absolute(path)?;
    for parent in path.ancestors().skip(1) {
        let m = std::fs::symlink_metadata(parent).map_err(|_| refused())?;
        if !m.is_dir() || m.uid() != owner || m.permissions().mode() & 0o022 != 0 {
            return Err(refused());
        }
    }
    Ok(())
}

pub(super) fn checked_file(path: &Path, owner: u32, private: bool) -> Result<File> {
    parents(path, owner)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| refused())?;
    let m = file.metadata().map_err(|_| refused())?;
    if !m.is_file()
        || m.uid() != owner
        || m.permissions().mode() & 0o6000 != 0
        || m.permissions().mode() & (if private { 0o027 } else { 0o022 }) != 0
    {
        return Err(refused());
    }
    Ok(file)
}

pub(super) fn bytes(file: File, cap: usize) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut result)
        .map_err(|_| refused())?;
    if result.len() > cap {
        return Err(refused());
    }
    Ok(result)
}

pub(super) fn load(path: Option<&Path>, reconcile: bool) -> Result<Profile> {
    let path = path.ok_or_else(|| {
        SdError::refused("native Ask settlement is disabled; no admitted profile")
    })?;
    let raw = bytes(checked_file(path, 0, true)?, 65536)?;
    let envelope: Envelope = serde_json::from_slice(&raw).map_err(|_| refused())?;
    if envelope.schema != "seeds-ask-native-port/v1" || (!envelope.enabled && !reconcile) {
        return Err(SdError::refused("native Ask settlement is disabled"));
    }
    envelope.admitted.ok_or_else(refused)
}

fn digest_ok(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub(super) fn verify_artifact(pin: &Artifact, owner: u32, cap: usize) -> Result<Vec<u8>> {
    if !digest_ok(&pin.sha256) {
        return Err(refused());
    }
    let body = bytes(checked_file(&pin.path, owner, false)?, cap)?;
    if crate::native::attest::sha256_hex(&body) != pin.sha256 {
        return Err(refused());
    }
    Ok(body)
}

impl Profile {
    pub fn validate(&self, code_owner: u32) -> Result<Option<String>> {
        let url = url::Url::parse(&self.endpoint).map_err(|_| refused())?;
        if url.scheme() != "https"
            || url.as_str() != format!("{}/", url.origin().ascii_serialization())
            || !url.username().is_empty()
            || url.password().is_some()
            || self.store_id.is_empty()
            || self.store_id.len() > 256
            || self.store_id.chars().any(char::is_control)
            || self.machine_principal.trim().is_empty()
            || self.machine_principal.len() > 256
            || self.machine_principal.chars().any(char::is_control)
            || self.custody_uid == 0
            || self.custody_uid != unsafe { libc::geteuid() }
        {
            return Err(refused());
        }
        absolute(&self.custody_directory)?;
        parents(&self.custody_directory, 0)?;
        if self.audit_location != "project" {
            return Err(refused());
        }
        let b = &self.reader["boundary"];
        if self.reader["endpoint"] != self.endpoint
            || self.reader["storeId"] != self.store_id
            || self.reader["operator"]["audience"] != self.store_id
            || b["itemGraph"] != self.graph
            || b["itemPrefix"] != crate::vocab::item_iri("")
            || b["revisionPredicate"] != crate::vocab::term::revision()
            || b["statusPredicate"] != crate::vocab::term::action_status()
            || b["openStatus"] != crate::vocab::schema("ActiveActionStatus")
        {
            return Err(refused());
        }
        if self.assets.len() != ASSETS.len() || ASSETS.iter().any(|p| !self.assets.contains_key(*p))
        {
            return Err(refused());
        }
        for name in ASSETS {
            verify_artifact(
                &Artifact {
                    path: self.code_root.join(name),
                    sha256: self.assets[name].clone(),
                },
                code_owner,
                262144,
            )?;
        }
        verify_artifact(&self.node, code_owner, 268435456)?;
        let ca = self
            .tls_ca
            .as_ref()
            .map(|pin| {
                String::from_utf8(verify_artifact(pin, code_owner, 32768)?).map_err(|_| refused())
            })
            .transpose()?;
        Ok(ca)
    }
}
