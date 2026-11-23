//! Disabled-by-default executable port for the Seeds-owned Ask transition.
//! No grant, binding, daemon, timer or deployment is selected by this module.
mod custody;
mod profile;
mod verifier;

use clap::Args as ClapArgs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;

use super::{
    cli::Cli,
    config::{Location, Resolved},
    remote::RemoteBackend,
    Outcome,
};
use crate::ask_settlement::{self, AdmittedSettlement, Request};
use crate::backend::Ctx;
use crate::error::{Result, SdError};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Root-owned operator profile; no default or environment fallback.
    #[arg(long)]
    pub profile: Option<PathBuf>,
    /// Revision-bound intent JSON, or '-' for bounded stdin.
    #[arg(long, default_value = "-")]
    pub request_file: String,
    /// Publish once through the admitted native verifier and Seeds CAS.
    #[arg(long, conflicts_with = "reconcile")]
    pub execute: bool,
    /// Read-only recovery; never reads a writer credential or reserves/sends.
    #[arg(long)]
    pub reconcile: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    graph: String,
    ask: String,
    ask_revision: u64,
    target: String,
    target_revision: u64,
    receipt: String,
}
impl Binding {
    fn request(&self) -> Request {
        Request {
            graph: self.graph.clone(),
            ask: self.ask.clone(),
            ask_revision: self.ask_revision,
            target: self.target.clone(),
            target_revision: self.target_revision,
            receipt: self.receipt.clone(),
        }
    }
    fn browser_json(&self) -> Value {
        json!({"graph":self.graph,"ask":self.ask,"askRevision":self.ask_revision,
            "target":self.target,"targetRevision":self.target_revision,"receipt":self.receipt})
    }
}
impl From<&Request> for Binding {
    fn from(r: &Request) -> Self {
        Self {
            graph: r.graph.clone(),
            ask: r.ask.clone(),
            ask_revision: r.ask_revision,
            target: r.target.clone(),
            target_revision: r.target_revision,
            receipt: r.receipt.clone(),
        }
    }
}

fn read_binding(path: &str) -> Result<Request> {
    let mut raw = Vec::new();
    let reader: Box<dyn Read> = if path == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(path).map_err(|_| profile::refused())?)
    };
    reader
        .take(8193)
        .read_to_end(&mut raw)
        .map_err(|_| profile::refused())?;
    if raw.len() > 8192 {
        return Err(profile::refused());
    }
    let value: Binding = serde_json::from_slice(&raw).map_err(|_| profile::refused())?;
    Ok(value.request())
}

pub(super) fn run(cli: &Cli, cfg: &Resolved, args: &Args) -> Result<Outcome> {
    // In particular: no backend, project creation, credential or stdin read.
    if !args.execute && !args.reconcile {
        return Err(SdError::refused(
            "native Ask settlement is disabled; explicit execute or reconcile required",
        ));
    }
    if cli.at.is_some() || cfg.pendant.is_some() {
        return Err(profile::refused());
    }
    let p = profile::load(args.profile.as_deref(), args.reconcile)?;
    let Location::Url(url) = &cfg.location else {
        return Err(profile::refused());
    };
    if url.trim_end_matches('/') != p.endpoint.trim_end_matches('/') || cfg.graph() != p.graph {
        return Err(SdError::refused(
            "native settlement endpoint/project differs from the admitted profile",
        ));
    }
    // This bearer-only machine path must not silently bypass a user's policy
    // requiring enrolled session signatures. It needs a dedicated admitted config.
    if !args.reconcile
        && (cfg.require_identity
            || cfg.signing.is_some()
            || cfg.actor_principal_mode == super::config::ActorPrincipalMode::Enforce)
    {
        return Err(SdError::refused(
            "native settler needs its separately admitted machine configuration",
        ));
    }
    let ca = p.validate(0)?;
    let intent = read_binding(&args.request_file)?;
    if intent.graph != p.graph {
        return Err(profile::refused());
    }
    let port = verifier::NativePort::new(p, intent.clone(), ca);
    // Verify before connecting the writer or reading its dedicated credential.
    port.read(&intent.receipt)?;
    let p = &port.profile;
    let credential = if args.reconcile {
        None
    } else {
        let file = p
            .writer_credential_file
            .as_ref()
            .ok_or_else(profile::refused)?;
        Some(custody::private_file(file, p.custody_uid)?)
    };
    let mut backend = RemoteBackend::connect_with_ca(
        &p.endpoint,
        &p.graph,
        credential,
        None,
        &[],
        p.tls_ca.as_ref().map(|pin| pin.path.as_path()),
        p.tls_ca.as_ref().map(|pin| pin.sha256.as_str()),
    )?
    .with_max_write_bytes(cfg.max_write_bytes)
    .with_max_write_clauses(cfg.max_write_clauses)
    .with_project_settler_audit();
    super::result::record_graph(&p.graph);
    let result = if args.reconcile {
        ask_settlement::reconcile(&backend, &intent, Some(&port))?
    } else {
        let ctx = Ctx {
            now: super::now(),
            actor: p.machine_principal.clone(),
            prefix: cfg.prefix.clone(),
            claims: super::claims(cli)?,
        };
        ask_settlement::settle(&mut backend, &ctx, &intent, Some(&port))?
    };
    let revision = intent
        .ask_revision
        .checked_add(1)
        .ok_or_else(profile::refused)?;
    backend.settler_audit_for(&intent.ask, revision)?;
    let value = json!({"graph":intent.graph,"ask":intent.ask,"askRevision":result.ask_revision,
        "target":intent.target,"targetRevision":result.target_revision,"receipt":result.receipt,
        "commentIndex":result.comment_index,"complete":true});
    Ok(super::ok(
        cli.wants_json(),
        value,
        "Atomic Ask settlement and audit read back".into(),
        vec![],
    ))
}

#[cfg(test)]
mod tests;
