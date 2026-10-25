//! Host-local observation of claimed actors under an authenticated signing key.
//! This is diagnostic attribution, never authentication or write acceptance.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::config::ActorPrincipalMode;
use crate::error::{Result, SdError};

const MAX_EVENTS: usize = 64;
const MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    actor: String,
    principal: String,
    verb: String,
    at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observations {
    mismatches_total: u64,
    events: Vec<Event>,
}

fn bounded(text: &str) -> String {
    text.chars().take(128).collect()
}

fn state_path() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".local/state"))
        })
        .ok_or_else(|| std::io::Error::other("no state directory"))?;
    Ok(base.join("seeds/actor-principal.json"))
}

fn record(path: &Path, event: Event) -> std::io::Result<u64> {
    let dir = path.parent().expect("observation path has parent");
    fs::create_dir_all(dir)?;
    let open = || {
        let mut o = OpenOptions::new();
        o.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        o
    };
    // A separate stable lock survives atomic replacement of the data file.
    let lock = open().open(dir.join("actor-principal.lock"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "observation counter lock busy after one second",
                ));
            }
            Err(error) => return Err(std::io::Error::other(error.to_string())),
        }
    }
    let mut bytes = Vec::new();
    open()
        .open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(std::io::Error::other("observation file exceeds bound"));
    }
    let mut state: Observations = if bytes.is_empty() {
        Observations::default()
    } else {
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)?
    };
    state.mismatches_total = state.mismatches_total.saturating_add(1);
    state.events.push(event);
    if state.events.len() > MAX_EVENTS {
        state.events.drain(..state.events.len() - MAX_EVENTS);
    }
    let bytes = serde_json::to_vec(&state).map_err(std::io::Error::other)?;
    let tmp = dir.join(format!("actor-principal.{}.tmp", std::process::id()));
    // A stale temporary is evidence of interrupted instrumentation, not a
    // reason to overwrite a path or refuse the caller's write.
    let mut options = open();
    options.create_new(true);
    let mut file = options.open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    Ok(state.mismatches_total)
}

/// Resolve the write actor while keeping the key principal a separate fact.
/// Observe records attempted mismatches, not proof of a committed write.
pub fn select(
    mode: ActorPrincipalMode,
    claim: Option<&str>,
    principal: &str,
    verb: &str,
    at: &str,
) -> Result<String> {
    let actor = claim.unwrap_or(principal);
    if actor == principal || mode == ActorPrincipalMode::Off {
        return Ok(actor.to_owned());
    }
    if mode == ActorPrincipalMode::Enforce {
        return Err(SdError::refused(
            "--actor/SEEDS_ACTOR differs from the authenticated principal",
        ));
    }
    let event = Event {
        actor: bounded(actor),
        principal: bounded(principal),
        verb: bounded(verb),
        at: bounded(at),
    };
    let result = state_path().and_then(|path| record(&path, event));
    match result {
        Ok(count) => eprintln!(
            "sd: actor/principal mismatch (observe): actor={:?} principal={:?} verb={:?} at={:?} mismatches_total={count}",
            bounded(actor), bounded(principal), bounded(verb), bounded(at)
        ),
        Err(error) => eprintln!(
            "sd: actor/principal mismatch (observe): actor={:?} principal={:?}; observation counter UNKNOWN ({error})",
            bounded(actor), bounded(principal)
        ),
    }
    Ok(actor.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_host_writers_do_not_lose_counter_increments() {
        let dir = std::env::temp_dir().join(format!("seeds-actor-parallel-{}", std::process::id()));
        assert!(!dir.exists());
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("actor-principal.json");
        let workers: Vec<_> = (0..8)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut committed = 0;
                    let mut busy = 0;
                    for _ in 0..10 {
                        match record(
                            &path,
                            Event {
                                actor: format!("worker-{n}"),
                                principal: "host-key".into(),
                                verb: "comment".into(),
                                at: "2026-01-01T00:00:00Z".into(),
                            },
                        ) {
                            Ok(_) => committed += 1,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                // Optional observations explicitly have a one-second
                                // contention budget; a busy attempt is not a commit.
                                busy += 1;
                            }
                            Err(error) => panic!("unexpected observation failure: {error}"),
                        }
                    }
                    (committed, busy)
                })
            })
            .collect();
        let (mut committed, mut busy) = (0, 0);
        for worker in workers {
            let (done, blocked) = worker.join().unwrap();
            committed += done;
            busy += blocked;
        }
        assert_eq!(committed + busy, 80);
        assert!(committed > 0, "no successful observation to verify");
        let state: Observations = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(state.mismatches_total, committed);
        assert_eq!(state.events.len(), (committed as usize).min(MAX_EVENTS));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn host_counter_retains_bounded_events_and_exact_total() {
        let dir = std::env::temp_dir().join(format!("seeds-actor-observe-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("actor-principal.json");
        assert!(!path.exists());
        for i in 1..=70 {
            assert_eq!(
                record(
                    &path,
                    Event {
                        actor: "worker-a".into(),
                        principal: "host-key".into(),
                        verb: "comment".into(),
                        at: i.to_string()
                    }
                )
                .unwrap(),
                i
            );
        }
        let state: Observations = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(state.mismatches_total, 70);
        assert_eq!(state.events.len(), MAX_EVENTS);
        assert_eq!(state.events[0].at, "7");
        assert_eq!(state.events.last().unwrap().actor, "worker-a");
        assert_eq!(state.events.last().unwrap().principal, "host-key");
        fs::remove_dir_all(dir).unwrap();
    }
}
