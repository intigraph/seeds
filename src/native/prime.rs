//! Stack prime orientation; all I/O remains inside the native boundary.
mod probe;
mod worker;

use super::{Cli, Outcome};
use clap::Args;
use serde_json::{json, Value};

const PRIMER: &str = include_str!("primer.md");

/// Native agent primer options.
#[derive(Debug, Args)]
pub struct PrimeArgs {
    /// Print five to ten lines for stack orientation
    #[arg(long)]
    brief: bool,
    /// Skip configuration and board I/O
    #[arg(long = "static")]
    offline: bool,
    /// Select a section by case-insensitive heading prefix
    #[arg(long)]
    section: Option<String>,
    /// Private read-only child protocol
    #[arg(long, hide = true)]
    live_worker: bool,
}

fn sections() -> Vec<&'static str> {
    PRIMER
        .lines()
        .filter_map(|s| s.strip_prefix("## "))
        .collect()
}

fn select(section: Option<&str>) -> std::result::Result<String, String> {
    let Some(section) = section else {
        return Ok(PRIMER.into());
    };
    let mut lines = Vec::new();
    let mut active = false;
    for line in PRIMER.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if active {
                break;
            }
            active = heading
                .to_lowercase()
                .starts_with(&section.trim().to_lowercase());
        }
        if active {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        Err(format!(
            "Unknown primer section {section:?}. Sections: {}",
            sections().join(", ")
        ))
    } else {
        Ok(lines.join("\n").trim_end().to_owned())
    }
}

pub(super) fn run(cli: &Cli, args: &PrimeArgs) -> Outcome {
    if args.live_worker {
        let live = worker::receive().map_or_else(
            || json!({"status": "unreadable", "summary": {}}),
            |cli| probe::run(&cli),
        );
        return Outcome {
            stdout: live.to_string(),
            ..Default::default()
        };
    }
    let text = match select(args.section.as_deref()) {
        Ok(text) => text,
        Err(error) => {
            return super::error_outcome(
                cli.wants_json(),
                &crate::error::SdError::usage(error),
                Vec::new(),
            )
        }
    };
    if cli.quiet {
        return Outcome::default();
    }
    let live = if args.offline {
        json!({"status": "skipped", "elapsed_ms": 0, "summary": {}})
    } else {
        worker::run(cli)
    };
    let stdout = if cli.wants_json() {
        let mut primer = json!({"sections": sections()});
        if !args.brief {
            primer["text"] = json!(text);
        }
        json!({"schema": "stack-prime/1", "tool": "sd", "version": env!("CARGO_PKG_VERSION"), "primer": primer,
            "status": if live["status"] == "ok" || live["status"] == "skipped" { "ok" } else { "degraded" }, "live": live}).to_string()
    } else if args.brief {
        brief(&live)
    } else if args.offline {
        text
    } else {
        format!(
            "{text}\n\n## Live state\n\nBoard status: {}.\n{}\n{}",
            live["status"].as_str().unwrap_or("unreadable"),
            live["summary"],
            live["hint"]
                .as_str()
                .unwrap_or("Read with `sd list --assignee <agent>` and `sd ready`.")
        )
    };
    Outcome {
        stdout,
        ..Default::default()
    }
}

fn brief(live: &Value) -> String {
    let mut lines = vec![format!(
        "sd {}: Seeds tracks work as versioned facts in Quipu.",
        env!("CARGO_PKG_VERSION")
    )];
    if live["status"] == "skipped" {
        let what = select(Some("What Seeds is")).expect("What section");
        lines.extend(
            what.lines()
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .take(5)
                .map(str::to_owned),
        );
    } else {
        let s = &live["summary"];
        lines.push(format!("Board: {} ({}).", s["board"], live["status"]));
        lines.push(format!("Graph: {}. Actor: {}.", s["graph"], s["actor"]));
        lines.push(format!(
            "Assigned active work: {}; ready count: {}.",
            human(&s["plate_total"]),
            human(&s["ready_count"])
        ));
        lines.push("Run `sd list --assignee <agent>` and `sd ready` for details.".into());
    }
    lines.push("Run `sd prime` for the full primer.".into());
    lines.join("\n")
}

fn human(value: &Value) -> String {
    if value.is_null() {
        "unknown".into()
    } else {
        value.to_string()
    }
}
