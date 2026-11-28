//! Intent is declared input, before clocks and generated IDs exist.
use super::*;
use crate::native::{
    cli::{Cli, Command, CommentsCommand},
    Outcome,
};

pub(super) struct Intent {
    pub digest: String,
    pub targets: Vec<String>,
    files: Vec<(PathBuf, String)>,
}
impl Intent {
    pub(super) fn check_files(&self) -> Result<()> {
        for (path, expected) in &self.files {
            if crate::pendant::sha256(&payload(path)?) != *expected {
                return Err(invalid("intent payload changed before send"));
            }
        }
        Ok(())
    }
}
fn payload(path: &std::path::Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(1_048_577).read_to_end(&mut data))
        .map_err(|_| invalid("intent payload unavailable"))?;
    if data.len() > 1_048_576 {
        return Err(invalid("intent payload byte bound"));
    }
    Ok(data)
}
fn normalize_file(
    value: &mut Value,
    file: &str,
    inline: &str,
    files: &mut Vec<(PathBuf, String)>,
) -> Result<()> {
    if let Some(path) = value[file].as_str() {
        if path == "-" {
            return Err(invalid("pilot requires inline or file payload, not stdin"));
        }
        let path = PathBuf::from(path);
        let bytes = payload(&path)?;
        let text = String::from_utf8(bytes.clone()).map_err(|_| invalid("intent payload UTF-8"))?;
        files.push((path, crate::pendant::sha256(&bytes)));
        value[inline] = Value::String(text);
        value[file] = Value::Null;
    }
    Ok(())
}
pub(super) fn make(cli: &Cli, actor: &str, project: &str) -> Result<Intent> {
    let mut files = Vec::new();
    let (args, targets) = match &cli.command {
        Command::Create(a) => (serde_json::to_value(a), vec![]),
        Command::Update(a) => (serde_json::to_value(a), a.ids.clone()),
        Command::Close(a) => (serde_json::to_value(a), a.ids.clone()),
        Command::Comments { command } => (
            serde_json::to_value(command),
            match command {
                CommentsCommand::Add { id, .. } | CommentsCommand::List { id } => vec![id.clone()],
            },
        ),
        _ if !cli.command.writes() => (Ok(json!({"parsed":format!("{:?}",cli.command)})), vec![]),
        _ => {
            return Err(invalid(
                "pilot supports create, update, close and comments writes only",
            ))
        }
    };
    let mut args = args.map_err(|_| invalid("intent encoding"))?;
    if matches!(cli.command, Command::Create(_) | Command::Update(_)) {
        normalize_file(&mut args, "description_file", "description", &mut files)?;
        // Bulk import and indirect structured payloads need their own normalizer.
        if args.get("file").is_some_and(|v| !v.is_null())
            || args["agent_context"]
                .as_str()
                .is_some_and(|s| s.starts_with('@'))
        {
            return Err(invalid(
                "pilot requires one create and inline agent context",
            ));
        }
    }
    if matches!(cli.command, Command::Create(_)) {
        if !args["title_flag"].is_null() {
            args["title"] = args["title_flag"].clone();
        }
        args.as_object_mut().unwrap().remove("title_flag");
        args.as_object_mut().unwrap().remove("silent");
    }
    if let Some(add) = args.get_mut("Add") {
        normalize_file(add, "file", "message", &mut files)?;
        if add["message"].is_null() {
            add["message"] = Value::String(
                add["text"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        add.as_object_mut().unwrap().remove("text");
    }
    let digest = crate::pendant::sha256(
        json!({"command":cli.command.name(),"args":args,"project":project,"actor":actor})
            .to_string()
            .as_bytes(),
    );
    Ok(Intent {
        digest: digest.strip_prefix("sha256:").unwrap().to_string(),
        targets,
        files,
    })
}
impl RemoteBackend {
    pub(crate) fn graph_set_prepare(
        &self,
        cli: &Cli,
        ctx: &crate::backend::Ctx,
    ) -> Result<Option<Outcome>> {
        let Some(mode) = &self.graph_sets else {
            return Ok(None);
        };
        if cli.at.is_some() {
            return Err(invalid("pilot has no historical read path"));
        }
        if !matches!(
            cli.command.name(),
            "create"
                | "update"
                | "close"
                | "comments add"
                | "comments list"
                | "show"
                | "list"
                | "ready"
                | "blocked"
                | "search"
                | "count"
        ) {
            return Err(invalid(
                "operation is outside the explicit pilot command set",
            ));
        }
        let intent = make(cli, &ctx.actor, &self.graph)?;
        let recovery = mode.recover(self, &intent)?;
        *mode.intent.borrow_mut() = Some(intent);
        if let Some(mut record) = recovery.same {
            if !recovery.prior.is_empty() {
                record["recovered_prior"] = json!(recovery.prior);
            }
            return Ok(Some(Outcome {
                code: 0,
                stdout: if cli.wants_json() {
                    format!("{record}\n")
                } else {
                    format!(
                        "Earlier attempt resolved: {} ({})\n",
                        record["item"].as_str().unwrap_or("batch"),
                        record["state"].as_str().unwrap_or("finished")
                    )
                },
                stderr: String::new(),
            }));
        }
        *mode.recovered_prior.borrow_mut() = recovery.prior.clone();
        if !recovery.prior.is_empty() {
            // Emitted before dispatch; preserves the recovery even if the new command refuses.
            eprintln!("{}", json!({"recovered_prior":recovery.prior}));
        }
        Ok(None)
    }
}

impl RemoteBackend {
    pub(crate) fn graph_set_finish(&self, cli: &Cli, mut outcome: Outcome) -> Result<Outcome> {
        if let Some(mode) = &self.graph_sets {
            let completed = mode.completed.borrow();
            let prior = mode.recovered_prior.borrow();
            if cli.wants_json() && (completed.is_some() || !prior.is_empty()) {
                let mut data: Value = serde_json::from_str(&outcome.stdout)
                    .map_err(|_| invalid("new command JSON output"))?;
                if completed.is_some() {
                    clear_main_tx(&mut data);
                }
                let mut report = json!({"data":data});
                if let Some(receipt) = completed.as_ref() {
                    report["completion"] = receipt.clone();
                }
                if !prior.is_empty() {
                    report["recovered_prior"] = json!(*prior);
                }
                outcome.stdout = format!("{report}\n");
            } else if let Some(receipt) = completed.as_ref() {
                outcome
                    .stderr
                    .push_str(&format!("\n{}", json!({"completion":receipt})));
            }
        }
        Ok(outcome)
    }
}
pub(super) fn clear_main_tx(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.contains_key("tx") {
                map.insert("tx".into(), Value::Null);
            }
        }
        Value::Array(values) => {
            for v in values {
                clear_main_tx(v);
            }
        }
        _ => {}
    }
}
