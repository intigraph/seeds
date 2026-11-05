//! One-shot task-manager transfers. All I/O stays in the native boundary.
mod github;
mod http;
mod records;

use super::{
    cli::{Cli, Command},
    config::Resolved,
    Outcome,
};
use crate::backend::{Backend, Ctx};
use crate::error::{Result, SdError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;

/// Explicit bounded selection and review controls, shared by both directions.
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Reviewed adapter TOML (no credentials).
    #[arg(long)]
    pub adapter_config: Option<std::path::PathBuf>,
    /// Seed IDs on export; external numeric IDs on import. At most 100.
    #[arg(long, value_delimiter = ',')]
    pub ids: Vec<String>,
    /// Apply the displayed one-shot plan. Default is read-only preview.
    #[arg(long, conflicts_with = "dry_run")]
    pub execute: bool,
    /// Explicit preview (the default).
    #[arg(long)]
    pub dry_run: bool,
    /// Explicitly choose source fields when a linked target differs.
    #[arg(long)]
    pub overwrite: bool,
    /// Named task-manager principal; separate from the Quipu credential.
    #[arg(long)]
    pub transfer_principal: Option<String>,
    /// Inherited anonymous private descriptor, supplied by the credential lane.
    #[arg(long)]
    pub transfer_credential_fd: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    pub endpoint: String,
    pub repository: Option<String>,
}

/// Provider-neutral transfer fields. No harness state, secrets or extra RDF.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Item {
    pub title: String,
    pub description: String,
    pub status: String,
    pub priority: u8,
    pub issue_type: String,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    pub parent: Option<String>,
    pub blocked_on: Vec<String>,
    pub comments: Vec<Note>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Note {
    pub key: String,
    pub author: String,
    pub at: String,
    pub text: String,
}
#[derive(Debug, Clone)]
pub(super) struct Remote {
    pub id: String,
    pub origin: Option<String>,
    pub link: String,
    pub item: Item,
}

pub(super) trait Adapter {
    fn find(&self, origin: &str) -> Result<Option<Remote>>;
    fn get(&self, id: &str) -> Result<Remote>;
    fn get_link(&self, link: &str) -> Result<Remote>;

    fn put(&self, origin: &str, item: &Item, current: Option<&Remote>) -> Result<Remote>;
    fn relate(
        &self,
        remote: &Remote,
        item: &Item,
        targets: &BTreeMap<String, Remote>,
        origin: &str,
    ) -> Result<()>;
}

pub fn is_target(to: Option<&str>) -> bool {
    matches!(to, Some("github"))
}
pub fn validate_selection(command: &Command) -> Result<()> {
    let args = match command {
        Command::Export(a) if !is_target(a.to.as_deref()) => &a.transfer,
        Command::Import(a) if a.from.is_none() => &a.transfer,
        _ => return Ok(()),
    };
    if args.adapter_config.is_some()
        || !args.ids.is_empty()
        || args.execute
        || args.dry_run
        || args.overwrite
        || args.transfer_principal.is_some()
        || args.transfer_credential_fd.is_some()
    {
        return Err(SdError::usage(
            "transfer flags require --to github or --from github",
        ));
    }
    Ok(())
}
pub fn selected(command: &Command) -> bool {
    matches!(command, Command::Export(a) if is_target(a.to.as_deref()))
        || matches!(command, Command::Import(a) if a.from.is_some())
}

pub(super) fn conflict(message: &str) -> SdError {
    SdError::conflict(message)
}

pub fn run(cli: &Cli, cfg: &Resolved, ctx: &Ctx, backend: &mut dyn Backend) -> Result<Outcome> {
    let (provider, args, exporting) = match &cli.command {
        Command::Export(a) => (a.to.as_deref().unwrap_or_default(), &a.transfer, true),
        Command::Import(a) => {
            if a.dir.is_some() || a.prefer.is_some() || a.replace {
                return Err(SdError::usage(
                    "external import cannot use pendant directory, --prefer or --replace",
                ));
            }
            (a.from.as_deref().unwrap_or_default(), &a.transfer, false)
        }
        _ => return Err(SdError::usage("not a task-manager transfer")),
    };
    if cli.at.is_some() {
        return Err(SdError::usage(
            "task-manager transfers require current state, not --at",
        ));
    }
    if args.ids.is_empty() || args.ids.len() > 100 {
        return Err(SdError::usage(
            "select 1..100 exact --ids; no implicit whole-board transfers",
        ));
    }
    let unique: std::collections::BTreeSet<_> = args.ids.iter().collect();
    if unique.len() != args.ids.len() {
        return Err(SdError::usage("duplicate transfer IDs"));
    }
    let path = args
        .adapter_config
        .as_ref()
        .ok_or_else(|| SdError::usage("--adapter-config required"))?;
    let mut data = String::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(65537).read_to_string(&mut data))
        .map_err(|_| SdError::usage("cannot read adapter configuration"))?;
    if data.len() > 65536 {
        return Err(SdError::usage("adapter configuration exceeds 64 KiB"));
    }
    let config: Config =
        toml::from_str(&data).map_err(|_| SdError::usage("invalid adapter configuration"))?;
    let client = http::Client::new(&config.endpoint, args)?;
    let adapter: Box<dyn Adapter> = match provider {
        "github" => Box::new(github::Github::new(config, client)?),
        _ => return Err(SdError::usage("unknown task-manager adapter")),
    };
    let operations = if exporting {
        records::export(
            backend,
            ctx,
            cfg.graph(),
            adapter.as_ref(),
            &args.ids,
            args.execute,
            args.overwrite,
        )?
    } else {
        records::import(
            backend,
            ctx,
            cfg.graph(),
            adapter.as_ref(),
            &args.ids,
            args.execute,
            args.overwrite,
        )?
    };
    let result = json!({"schema":"seeds.transfer/1", "adapter":provider,
        "direction":if exporting {"export"} else {"import"},
        "dry_run":!args.execute,"operations":operations});
    Ok(super::ok(
        cli.json,
        result.clone(),
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        vec![],
    ))
}

pub(super) fn required(value: &Value, name: &str) -> Result<String> {
    value[name]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| SdError::refused("task-manager response lacks a required field"))
}

/// Decode only bounded UTF-8 origin markers, never arbitrary provider metadata.
pub(super) fn decode_origin(text: &str, start: &str, end: &str) -> Option<String> {
    let hex = text.split_once(start)?.1.split_once(end)?.0;
    if hex.len() > 8192 || hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests;
