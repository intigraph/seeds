//! Bounded maintained-summary transport. Refusals never trigger aggregate or
//! whole-graph reads; the existing vocabulary preflight remains separate.
use super::*;
use crate::project_summary::ProjectSummary;
use serde::Deserialize;
use std::io::Read;
const LIMIT: usize = 65_536;

#[derive(Deserialize)]
struct Response {
    contract: String,
    graph: String,
    summary: ProjectSummary,
}
fn refused() -> SdError {
    SdError::refused("invalid or unsupported maintained project summary; no scan fallback")
}
fn bounded_body(response: ureq::Response) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| SdError::failed(format!("cannot read project summary: {error}")))?;
    if bytes.len() > LIMIT {
        return Err(SdError::refused(
            "project summary exceeds 64 KiB; no scan fallback",
        ));
    }
    Ok(bytes)
}
fn clock(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 30
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'.',
            29 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        })
        && crate::engine::parse_until(value, value).is_ok()
}
fn sum(mut values: impl Iterator<Item = u64>) -> Option<u64> {
    values.try_fold(0_u64, |total, value| total.checked_add(value))
}
fn decode(bytes: &[u8], graph: &str) -> Result<ProjectSummary> {
    let raw: Json = serde_json::from_slice(bytes).map_err(|_| refused())?;
    // Option<T> accepts a missing JSON member; this contract requires explicit
    // unsupported fields and an explicit validity bound instead.
    if raw["summary"].get("valid_until").is_none()
        || raw["summary"]["counts"].get("semantic_transaction") != Some(&Json::Null)
        || raw["summary"]["counts"].get("dynamic_statistics") != Some(&Json::Null)
    {
        return Err(refused());
    }
    let response: Response = serde_json::from_value(raw).map_err(|_| refused())?;
    let summary = response.summary;
    let coverage = &summary.coverage;
    let counts = &summary.counts;
    if response.contract != "project-summary/v1"
        || response.graph != graph
        || summary.project_iri != graph
        || summary.ephemeral_iri != vocab::ephemeral_graph(graph)
        || coverage.schema_id != "project-summary-coverage-v5"
        || coverage.project <= 0
        || coverage.ephemeral <= 0
        || coverage.project == coverage.ephemeral
        || coverage.generation < 0
        || coverage.store_id.is_empty()
        || coverage.store_id.len() > 128
        || !clock(&summary.evaluated_at)
        || summary
            .valid_until
            .as_deref()
            .is_some_and(|until| !clock(until) || until <= summary.evaluated_at.as_str())
        || !matches!(
            counts.maintenance.as_str(),
            "reconcile-only-not-write-maintained" | "transactional-static-counts"
        )
    {
        return Err(refused());
    }
    for total in [
        sum(counts.by_status.values().copied()),
        sum(counts.by_kind.values().copied()),
        sum(counts.by_priority.values().copied()),
        sum(counts.by_assignee.values().copied()),
    ] {
        if total != Some(counts.total) {
            return Err(refused());
        }
    }
    if counts.by_label.values().any(|count| *count > counts.total)
        || sum(counts.by_label.values().copied()).is_none_or(|total| total < counts.total)
    {
        return Err(refused());
    }
    Ok(summary)
}
impl RemoteBackend {
    pub(super) fn maintained_summary(&self) -> Result<ProjectSummary> {
        let body = json!({"graph":self.graph}).to_string();
        let response = retry_read(|| {
            quipu_read_request(
                self.agent.post(&format!("{}/project/summary", self.base)),
                "project_summary",
            )
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .send_string(&body)
            .map_err(Box::new)
        });
        let bytes = match response {
            Ok(response) => bounded_body(response)?,
            Err(error) => {
                return Err(match *error {
                    ureq::Error::Transport(error) => transport(&self.base, &error),
                    ureq::Error::Status(code, response) => {
                        let bytes = bounded_body(response)?;
                        status_error("project summary", code, &String::from_utf8_lossy(&bytes))
                    }
                })
            }
        };
        decode(&bytes, &self.graph)
    }
}
