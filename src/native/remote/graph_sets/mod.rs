//! Explicit, default-off graph-set client. No legacy fallback or write replay.
use super::{quipu_read_request, quipu_request, RemoteBackend};
use crate::error::{ErrorKind, Result, SdError};
use crate::model::Fact;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

mod attempt;
mod intent;
mod journal;
mod proof;
#[cfg(test)]
mod tests;
mod transport;
use journal::Journal;

const PROFILE_ENV: &str = "SEEDS_GRAPH_SET_PROFILE";
const REQUEST_CONTRACT: &str = "quipu.knot-completion/v2";
const STATUS_CONTRACT: &str = "quipu.update-completion/v1";
const ATTEMPT_CONTRACT: &str = "seeds.graph-set-attempt/v1";

fn invalid(why: &str) -> SdError {
    SdError::refused(format!("graph-set mode: {why}; no legacy fallback"))
}
fn hex_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn new_id() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| invalid("write ID entropy unavailable"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 2048 && !value.chars().any(char::is_control)
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Profile {
    pub base: String,
    pub project: String,
    pub store_id: String,
    pub log_epoch: String,
    /// Operator-declared expected credential descriptor, never ctx.actor.
    pub principal: String,
    pub credential_id: String,
    pub denied_control_graph: String,
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
}
impl Profile {
    fn members(&self) -> Vec<String> {
        vec![
            self.project.clone(),
            crate::vocab::ephemeral_graph(&self.project),
            super::provenance_graph(&self.project),
        ]
    }
    fn validate(&self, base: &str, project: &str) -> Result<()> {
        if self.base != base
            || self.project != project
            || [
                &self.store_id,
                &self.log_epoch,
                &self.principal,
                &self.credential_id,
            ]
            .iter()
            .any(|v| !identifier(v))
            || !url::Url::parse(&self.project)
                .is_ok_and(|u| u.username().is_empty() && u.password().is_none())
            || !url::Url::parse(&self.denied_control_graph)
                .is_ok_and(|u| u.username().is_empty() && u.password().is_none())
            || self.members().contains(&self.denied_control_graph)
        {
            return Err(invalid(
                "profile endpoint, project, lineage, credential or control mismatch",
            ));
        }
        Ok(())
    }
    fn state_dir(&self) -> Result<PathBuf> {
        if let Some(path) = &self.state_dir {
            return Ok(path.clone());
        }
        let root = match std::env::var_os("XDG_STATE_HOME") {
            Some(root) => PathBuf::from(root),
            None => PathBuf::from(
                std::env::var_os("HOME").ok_or_else(|| invalid("state HOME unavailable"))?,
            )
            .join(".local/state"),
        };
        Ok(root.join("seeds/attempts"))
    }
}

pub(super) struct Mode {
    profile: Profile,
    journal: Journal,
    credential_sha256: String,
    intent: std::cell::RefCell<Option<intent::Intent>>,
    recovered_prior: std::cell::RefCell<Vec<Value>>,
    completed: std::cell::RefCell<Option<Value>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    seeds: Vec<Vec<Fact>>,
    comments: Vec<Vec<Fact>>,
    delete_seeds: Vec<(String, u64)>,
    delete_comments: Vec<(String, u64)>,
    record: Vec<(String, Vec<Fact>)>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    schema: String,
    write_id: String,
    base: String,
    project: String,
    store_id: String,
    log_epoch: String,
    principal: String,
    credential_id: String,
    credential_sha256: String,
    intent_digest: String,
    item_ids: Vec<String>,
    update: String,
    form: String,
    body_sha256: String,
    phase: String,
    expected: Expected,
}
impl Attempt {
    fn validate(&self) -> Result<()> {
        if self.schema != ATTEMPT_CONTRACT
            || !hex_id(&self.write_id)
            || !hex_id(&self.intent_digest)
            || self.body_sha256 != crate::pendant::sha256(self.form.as_bytes())
            || !matches!(
                self.phase.as_str(),
                "prepared"
                    | "unknown"
                    | "admitted"
                    | "committed"
                    | "finished"
                    | "stopped"
                    | "sealed"
            )
            || self.item_ids.is_empty()
            || self.item_ids.len() > 128
            || self.item_ids.iter().any(|id| !identifier(id))
            || [
                &self.base,
                &self.project,
                &self.store_id,
                &self.log_epoch,
                &self.principal,
                &self.credential_id,
                &self.credential_sha256,
            ]
            .iter()
            .any(|v| !identifier(v))
        {
            return Err(invalid("invalid attempt envelope"));
        }
        let fields: Vec<_> = url::form_urlencoded::parse(self.form.as_bytes()).collect();
        let updates: Vec<_> = fields.iter().filter(|(k, _)| k == "update").collect();
        if updates.len() != 1 || updates[0].1 != self.update {
            return Err(invalid("attempt text/form mismatch"));
        }
        Ok(())
    }
    fn unknown(&self, state: &str, local_tx: Option<u64>) -> SdError {
        SdError::new(
            ErrorKind::Indeterminate,
            "graph-set write custody UNKNOWN; retained exact attempt, no new write",
        )
        .with_details(
            json!({"schema":"seed-write-outcome/v1","state":state,"write_id":self.write_id,
                "owner":{"store_id":self.store_id,"log_epoch":self.log_epoch},"local_tx":local_tx,
                "item_ids":self.item_ids,"outcome":"UNKNOWN"}),
        )
    }
}

impl Mode {
    pub(super) fn from_env(remote: &RemoteBackend) -> Result<Option<Self>> {
        let Some(path) = std::env::var_os(PROFILE_ENV) else {
            return Ok(None);
        };
        if path.is_empty() {
            return Err(invalid("empty profile selection"));
        }
        let bytes = journal::read_private(&PathBuf::from(path), 65536)?;
        let profile: Profile =
            serde_json::from_slice(&bytes).map_err(|_| invalid("invalid user profile"))?;
        Self::open(remote, profile).map(Some)
    }
    fn open(remote: &RemoteBackend, profile: Profile) -> Result<Self> {
        profile.validate(&remote.base, &remote.graph)?;
        if remote.signer.is_some() || remote.token.as_ref().is_none_or(String::is_empty) {
            return Err(invalid(
                "pilot requires one named bearer; signing mode is not downgraded",
            ));
        }
        let journal = Journal::open(profile.state_dir()?)?;
        let credential_sha256 = crate::pendant::sha256(remote.token.as_ref().unwrap().as_bytes());
        let mode = Self {
            profile,
            journal,
            credential_sha256,
            intent: Default::default(),
            recovered_prior: Default::default(),
            completed: Default::default(),
        };
        mode.preflight(remote)?;
        Ok(mode)
    }
}

impl Mode {
    pub(super) fn members(&self) -> Vec<String> {
        self.profile.members()
    }
}
