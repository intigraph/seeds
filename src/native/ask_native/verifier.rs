//! The selected immutable program reads native evidence; stdin is only intent.
use std::cell::RefCell;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    custody,
    profile::{refused, Profile, ASSETS},
    Binding,
};
use crate::ask_settlement::{AdmittedSettlement, Answer, Request};
use crate::error::{Result, SdError};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Verified {
    schema: String,
    binding: Binding,
    choice: String,
    actor: String,
    recorded_at: String,
    evidence: Value,
}

pub(super) struct NativePort {
    pub profile: Profile,
    pub intent: Request,
    pub ca: Option<String>,
    verified: RefCell<Option<Verified>>,
}

fn output(profile: &Profile, body: &Value) -> Result<Vec<u8>> {
    let raw = serde_json::to_vec(body).map_err(|_| refused())?;
    if raw.len() > 65536 {
        return Err(refused());
    }
    let mut child = Command::new(&profile.node.path)
        .arg(profile.code_root.join(ASSETS[0]))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("HOME", &profile.custody_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| refused())?;
    let stdout = child.stdout.take().ok_or_else(refused)?;
    let stderr = child.stderr.take().ok_or_else(refused)?;
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(262145).read_to_end(&mut bytes).map(|_| bytes)
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.take(2049).read_to_end(&mut bytes).map(|_| bytes)
    });
    let mut stdin = child.stdin.take().ok_or_else(refused)?;
    let input = std::thread::spawn(move || stdin.write_all(&raw));
    let began = Instant::now();
    let success = loop {
        if began.elapsed() > Duration::from_secs(8) {
            break false;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break false,
        }
    };
    // No subprocesses are launched by the verifier; reap this one on every path.
    if !success {
        let _ = child.kill();
    }
    let _ = child.wait();
    let sent = input.join().map_err(|_| refused())?.is_ok();
    let stdout = out.join().map_err(|_| refused())?.map_err(|_| refused())?;
    let stderr = err.join().map_err(|_| refused())?.map_err(|_| refused())?;
    if !success || !sent || stdout.len() > 262144 || stderr.len() > 2048 {
        return Err(SdError::refused(
            "native receipt verifier refused or timed out; no send admitted",
        ));
    }
    Ok(stdout)
}

impl NativePort {
    pub fn new(profile: Profile, intent: Request, ca: Option<String>) -> Self {
        Self {
            profile,
            intent,
            ca,
            verified: RefCell::new(None),
        }
    }
}

impl AdmittedSettlement for NativePort {
    fn read(&self, receipt: &str) -> Result<Answer> {
        if receipt != self.intent.receipt {
            return Err(refused());
        }
        if self.verified.borrow().is_none() {
            let raw = output(
                &self.profile,
                &json!({"schema":"seeds-native-verifier/v1",
                "binding": Binding::from(&self.intent).browser_json(), "reader":self.profile.reader, "tlsCaPem":self.ca}),
            )?;
            // This private pipe is from the hash-pinned program. There is no CLI
            // flag or input file that supplies a Verified/Answer object.
            let result: Verified = serde_json::from_slice(&raw).map_err(|_| refused())?;
            if result.schema != "seeds-native-answer/v1"
                || result.binding.request() != self.intent
                || result.evidence["receipt_iri"] != self.intent.receipt
                || result.evidence["verified"] != true
                || result.evidence["receipt"]["challenge"]["audience"] != self.profile.store_id
                || result.actor != self.profile.reader["operator"]["actor"]
            {
                return Err(refused());
            }
            *self.verified.borrow_mut() = Some(result);
        }
        let borrowed = self.verified.borrow();
        let answer = borrowed.as_ref().ok_or_else(refused)?;
        Ok(Answer {
            binding: answer.binding.request(),
            choice: answer.choice.clone(),
            actor: answer.actor.clone(),
            recorded_at: answer.recorded_at.clone(),
        })
    }

    fn reserve_send(&self, request: &Request) -> Result<()> {
        if request != &self.intent {
            return Err(refused());
        }
        let borrowed = self.verified.borrow();
        let proof = borrowed.as_ref().ok_or_else(refused)?;
        // Code/profile changes must NEVER yield a new send slot for this intent.
        let identity = json!({"store_id":self.profile.store_id, "binding":Binding::from(request)});
        let record = json!({"schema":"seeds-ask-send-intent/v1", "state":"SEND_RESERVED",
            "identity":identity, "verifier_assets":self.profile.assets,
            "evidence":proof.evidence, "choice":proof.choice, "actor":proof.actor, "recorded_at":proof.recorded_at});
        custody::reserve(
            &self.profile.custody_directory,
            self.profile.custody_uid,
            &identity,
            &record,
        )
    }
}
