//! Opt-in, body-free failure envelopes. Legacy JSON and exit codes stay intact.
use super::{
    cli::{Cli, Command, CommentsCommand},
    run_legacy, Outcome,
};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};

thread_local! {
    static PHASE: Cell<&'static str> = const { Cell::new("PREPARE") };
    static SUBMITTED: Cell<bool> = const { Cell::new(false) };
    static GRAPH: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(super) fn record_graph(graph: &str) {
    GRAPH.with(|g| *g.borrow_mut() = Some(graph.to_owned()));
}

pub(super) fn request_phase(write: bool) {
    if write {
        SUBMITTED.with(|s| s.set(true));
    }
    PHASE.with(|p| {
        p.set(if write {
            "WRITE"
        } else if SUBMITTED.with(Cell::get) {
            "READBACK"
        } else {
            "READ"
        })
    });
}

struct Trace(&'static str, bool, Option<String>);
impl Trace {
    fn new() -> Self {
        Self(
            PHASE.with(|p| p.replace("PREPARE")),
            SUBMITTED.with(|p| p.replace(false)),
            GRAPH.with(|g| g.replace(None)),
        )
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        PHASE.with(|p| p.set(self.0));
        SUBMITTED.with(|p| p.set(self.1));
        GRAPH.with(|g| *g.borrow_mut() = self.2.take());
    }
}

fn tasks(command: &Command) -> Vec<String> {
    let ids = match command {
        Command::Show(a) => a.ids.clone(),
        Command::Update(a) => a.ids.clone(),
        Command::Close(a) => a.ids.clone(),
        Command::Delete(a) => a.ids.clone(),
        Command::Comments {
            command: CommentsCommand::Add { id, .. } | CommentsCommand::List { id },
        } => vec![id.clone()],
        _ => vec![],
    };
    ids.into_iter()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 256
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        })
        .collect()
}

pub(super) fn usage_failure(code: i32) -> Outcome {
    Outcome { code, stdout: json!({"schema":"tracker-result/v1", "ok":false,
        "operation_id":operation_id(), "operation":null, "tasks":[], "graph":null,
        "phase":"INPUT", "mutation_outcome":"NOT_WRITTEN", "exit_code":code, "recovery":null,
        "data":null, "error":{"category":"VALIDATION","code":"USAGE","http_status":null,
        "message":"Invalid command; arguments omitted"}, "reconcile":"Correct input; no automatic retry"}).to_string(),
        stderr:"sd: invalid command; see result envelope".into() }
}

/// Identity is random per invocation, never a duplicate-write/retry token.
fn operation_id() -> Option<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn category(code: i32, error: &Value) -> &'static str {
    if error["details"]["category"] == "MISSING_INPUT" {
        return "MISSING_INPUT";
    }
    if error["details"]["category"] == "MALFORMED_RESPONSE" {
        return "MALFORMED_RESPONSE";
    }
    if error["details"]["category"] == "TIMEOUT" {
        return "TIMEOUT";
    }
    if error["details"]["category"] == "PARTIAL" {
        return "PARTIAL";
    }
    match error["details"]["http_status"].as_u64() {
        Some(401) => return "AUTH",
        Some(403) => return "POLICY",
        Some(408) => return "TIMEOUT",
        Some(_) => return "TRANSPORT",
        _ => (),
    }
    match code {
        2 => "VALIDATION",
        3 => "NOT_FOUND",
        4 => "CONFLICT",
        5 => "POLICY",
        6 => "CONFIG",
        7 => "TRANSPORT",
        8 => "INDETERMINATE",
        _ => "UNKNOWN",
    }
}

fn recovery(error: &Value) -> Value {
    let details = &error["details"];
    let Some(id) = details["operation_id"].as_str().filter(|id| {
        id.strip_prefix("urn:seeds:write:").is_some_and(|suffix| {
            suffix.len() == 24 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
        })
    }) else {
        return Value::Null;
    };
    let graph = details["provenance_graph"].as_str().filter(|g| {
        url::Url::parse(g)
            .is_ok_and(|u| u.username().is_empty() && u.password().is_none() && u.query().is_none())
    });
    let items: Vec<&str> = details["item_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 256
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        })
        .collect();
    json!({"operation_id":id,"provenance_graph":graph,"item_ids":items})
}

pub(super) fn run(cli: &Cli) -> Outcome {
    capture(cli, cli.graph.clone(), || run_legacy(cli))
}

pub(super) fn capture(
    cli: &Cli,
    graph: Option<String>,
    execute: impl FnOnce() -> Outcome,
) -> Outcome {
    let _trace = Trace::new();
    let id = operation_id();
    let mut o = execute();
    let graph = GRAPH.with(|g| g.borrow().clone()).or(graph).filter(|g| {
        url::Url::parse(g)
            .is_ok_and(|u| u.username().is_empty() && u.password().is_none() && u.query().is_none())
    });
    let value: Value = match serde_json::from_str(&o.stdout) {
        Ok(value) => value,
        Err(_) if o.code == 0 => {
            o.code = 1;
            json!({"error":{"code":"FAILED","details":{"category":"MALFORMED_RESPONSE"}}})
        }
        Err(_) => Value::Null,
    };
    let error = &value["error"].clone();
    let planned = match &cli.command {
        Command::Create(a) => a.dry_run,
        Command::Delete(a) => a.dry_run,
        _ => false,
    };
    let writes = cli.command.writes() && !planned && !cli.credential_admission;
    let mutation = if !writes {
        "NOT_APPLICABLE"
    } else if o.code == 0 {
        "COMMITTED"
    } else if error["details"]["phase"] == "INPUT"
        || (!SUBMITTED.with(Cell::get) && PHASE.with(Cell::get) == "READ")
    {
        "NOT_WRITTEN"
    } else {
        "UNKNOWN"
    };
    let result = json!({
        "schema": "tracker-result/v1", "ok": o.code == 0,
        "operation_id": id, "operation": cli.command.name(),
        "tasks": tasks(&cli.command), "graph": graph,
        // A read-before-write can fail before mutation. Do not guess its boundary.
        "phase": if error["details"]["phase"] == "INPUT" { "INPUT" } else { PHASE.with(Cell::get) },
        "mutation_outcome": mutation, "exit_code": o.code,
        "recovery": recovery(error),
        "data": if o.code == 0 { value } else { Value::Null },
        "error": if o.code == 0 { Value::Null } else { json!({
            "category": category(o.code, error),
            "code": error["code"].as_str().unwrap_or("UNKNOWN"),
            "http_status": error["details"]["http_status"],
            "message": "Operation failed; private diagnostics omitted"
        }) },
        "reconcile": if writes && o.code != 0 {
            "Read exact task status and operation evidence before any retry; never replay a comment blindly"
        } else { "No automatic retry or store/credential change" }
    });
    Outcome {
        code: o.code,
        stdout: result.to_string(),
        stderr: if o.code == 0 {
            String::new()
        } else {
            "sd: operation failed; see result envelope".into()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn result_flag_requests_json_without_opening_any_store() {
        let cli = Cli::try_parse_from(["sd", "--result-json", "show", "absent"]).unwrap();
        assert!(cli.wants_json());
        assert_eq!(tasks(&cli.command), vec!["absent"]);
        assert_eq!(operation_id().unwrap().len(), 32);
    }

    #[test]
    fn categories_do_not_echo_server_bodies() {
        for (status, expected) in [
            (401, "AUTH"),
            (403, "POLICY"),
            (408, "TIMEOUT"),
            (503, "TRANSPORT"),
        ] {
            assert_eq!(
                category(
                    1,
                    &json!({"details":{"http_status":status},"message":"Bearer private"})
                ),
                expected
            );
        }
        assert_eq!(category(8, &Value::Null), "INDETERMINATE");
    }
}
