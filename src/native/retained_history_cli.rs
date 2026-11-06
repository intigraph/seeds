//! Explicit native access to retained facts; never substitutes for `history`.
use super::cli::{Cli, Command};
use super::config::{Location, Resolved};
use super::remote::history::HistoryRequest;
use super::remote::RemoteBackend;
use super::Outcome;
use crate::error::{Result, SdError};
use std::time::Duration;

pub(super) fn selected(cli: &Cli) -> bool {
    matches!(
        cli.command,
        Command::History {
            retained_facts: true,
            ..
        }
    )
}

/// Refuse invalid selections before connecting or opening a local store.
pub(super) fn validate(cli: &Cli, cfg: &Resolved) -> Result<()> {
    if !selected(cli) {
        return Ok(());
    }
    let Command::History {
        source_store_id, ..
    } = &cli.command
    else {
        unreachable!()
    };
    if !matches!(cfg.location, Location::Url(_)) || cli.at.is_some() || !cli.json {
        return Err(SdError::usage(
            "history --retained-facts requires a remote backend and --json; it takes no --at",
        ));
    }
    if !cfg.graph.as_ref().is_some_and(|graph| {
        graph != "urn:quipu:graph:root" && oxiri::Iri::parse(graph.as_str()).is_ok()
    }) {
        return Err(SdError::usage(
            "history --retained-facts requires an explicitly configured named graph; no project creation or ROOT fallback",
        ));
    }
    if source_store_id
        .as_ref()
        .is_none_or(|id| id.trim().is_empty() || id.len() > 256 || id.chars().any(char::is_control))
    {
        return Err(SdError::usage(
            "history --retained-facts requires --source-store-id from trusted operator configuration; no response-based enrollment",
        ));
    }
    Ok(())
}

pub(super) fn run(cli: &Cli, cfg: &Resolved, remote: &RemoteBackend) -> Result<Outcome> {
    validate(cli, cfg)?;
    let Command::History {
        id,
        source_store_id,
        comment,
        ..
    } = &cli.command
    else {
        unreachable!()
    };
    let subject = match comment {
        Some(index) => crate::vocab::comment_iri(id, *index),
        None => crate::vocab::item_iri(id),
    };
    let retained = remote.retained_history(
        &HistoryRequest {
            iri: subject.clone(),
            graph: cfg.graph().to_string(),
            source_store_id: source_store_id.clone().expect("validated source"),
            expected_generation: None,
            since_tx: None,
            until_tx: None,
            max_entries: 10_000,
            max_bytes: 4 * 1024 * 1024,
        },
        Duration::from_secs(30),
    )?;
    if retained.response()["count"].as_u64() == Some(0) {
        return Err(SdError::refused(
            "no retained facts for the exact subject; existence and history are UNKNOWN; no fallback",
        ));
    }
    // Preserve the original source bytes alongside the parsed response. A
    // complete retained range is not every-version or historical-policy proof.
    let source_wire = std::str::from_utf8(retained.source_bytes())
        .map_err(|_| SdError::refused("retained history source is not UTF-8; no fallback"))?;
    let value = serde_json::json!({
        "mode": "retained-main-facts",
        "meaning": "bounded retained facts, not continuous version history or complete comment-family coverage",
        "id": id,
        "subject": subject,
        "source_response": retained.response(),
        "source_response_utf8": source_wire,
    });
    Ok(super::ok(true, value, String::new(), vec![]))
}
