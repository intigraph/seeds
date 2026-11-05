//! Bounded IPC: no credentials in argv, no temporary files, no orphan worker.
use super::super::Cli;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const CAP: u64 = 64 * 1024;
const INPUT_CAP: usize = 16 * 1024;
const BUDGET: Duration = Duration::from_millis(750);

struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn input(cli: &Cli) -> Vec<String> {
    let mut args = vec!["sd".into(), "prime".into()];
    for (flag, value) in [
        ("--store", &cli.store),
        ("--quipu", &cli.quipu),
        ("--graph", &cli.graph),
        ("--actor", &cli.actor),
    ] {
        if let Some(value) = value {
            args.extend([flag.into(), value.clone()]);
        }
    }
    if let Some(tx) = cli.at {
        args.extend(["--at".into(), tx.to_string()]);
    }
    args
}

pub(super) fn receive() -> Option<Cli> {
    use clap::Parser;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(INPUT_CAP as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > INPUT_CAP {
        return None;
    }
    let args: Vec<String> = serde_json::from_slice(&bytes).ok()?;
    let cli = Cli::try_parse_from(args).ok()?;
    matches!(cli.command, super::super::cli::Command::Prime(_)).then_some(cli)
}

pub(super) fn run(cli: &Cli) -> Value {
    let started = Instant::now();
    let result = collect(cli, started);
    let mut live = result.unwrap_or_else(|| json!({"status": "unreadable", "summary": super::probe::empty_summary(cli.at), "hint": "Run `sd where --json` and `sd doctor`."}));
    live["elapsed_ms"] = json!(started.elapsed().as_millis());
    live
}

fn collect(cli: &Cli, started: Instant) -> Option<Value> {
    let bytes = serde_json::to_vec(&input(cli)).ok()?;
    if bytes.len() > INPUT_CAP {
        return None;
    }
    let mut child = Reap(
        Command::new(std::env::current_exe().ok()?)
            .args(["prime", "--live-worker"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?,
    );
    child.0.stdin.take()?.write_all(&bytes).ok()?;
    let stdout = child.0.stdout.take()?;
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        let mut data = Vec::new();
        let result = stdout.take(CAP + 1).read_to_end(&mut data).map(|_| data);
        let _ = send.send(result);
    });
    loop {
        if started.elapsed() >= BUDGET {
            return Some(
                json!({"status": "partial", "summary": super::probe::empty_summary(cli.at), "hint": "Board summary deadline reached; run `sd where --json` and retry `sd prime`."}),
            );
        }
        match recv.try_recv() {
            Ok(Ok(data)) => {
                if data.len() as u64 > CAP {
                    return None;
                }
                let mut value: Value = serde_json::from_slice(&data).ok()?;
                if !matches!(
                    value["status"].as_str(),
                    Some("ok" | "absent" | "unreadable" | "unreachable")
                ) {
                    return None;
                }
                value["elapsed_ms"] = json!(started.elapsed().as_millis());
                return Some(value);
            }
            Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => return None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        // Never return solely because the process exited: the pipe reader
        // may still be draining its bounded final response.
        let _ = child.0.try_wait().ok()?;
        std::thread::sleep(Duration::from_millis(5));
    }
}
