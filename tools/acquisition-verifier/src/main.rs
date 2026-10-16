//! One bounded verification operation. No server, store or network access.
use quipu::acquisition::composed::{Payload, Request};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::{self, Read};

const SOURCE: &str = "0f23ef3be3cc6ed32d62dacd4cf7c0986fc803bb";
const MAX_INPUT: u64 = 268_500_992;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    schema: String,
    request_bytes: usize,
    wire_bytes: usize,
}

struct Frame<'a> {
    request: &'a [u8],
    wire: &'a [u8],
}

fn unframe(input: &[u8]) -> Result<Frame<'_>, Box<dyn std::error::Error>> {
    let newline = input
        .iter()
        .take(4096)
        .position(|b| *b == b'\n')
        .ok_or("header limit")?;
    let header: Header = serde_json::from_slice(&input[..newline])?;
    if header.schema != "seeds-acquisition-input/v1"
        || header.request_bytes == 0
        || header.request_bytes > 65_536
        || header.wire_bytes == 0
        || header.wire_bytes > 268_435_456
    {
        return Err("frame bounds".into());
    }
    let start = newline + 1;
    let split = start
        .checked_add(header.request_bytes)
        .ok_or("frame overflow")?;
    let end = split
        .checked_add(header.wire_bytes)
        .ok_or("frame overflow")?;
    if end != input.len() {
        return Err("frame incomplete or extra bytes".into());
    }
    Ok(Frame {
        request: &input[start..split],
        wire: &input[split..end],
    })
}

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn verify() -> Result<(), Box<dyn std::error::Error>> {
    limits()?;
    let mut input = Vec::new();
    io::stdin().take(MAX_INPUT + 1).read_to_end(&mut input)?;
    if input.len() as u64 > MAX_INPUT {
        return Err("input byte limit".into());
    }
    let Frame {
        request: request_bytes,
        wire,
    } = unframe(&input)?;
    let request: Request = serde_json::from_slice(request_bytes)?;
    let payload: Payload = serde_json::from_slice(wire)?;
    // The authoritative verifier, not the DTO decoder, admits the payload.
    payload.verify(&request)?;
    println!(
        "{}",
        json!({
            "schema":"seeds-acquisition-verifier/v1",
            "verifier_source":SOURCE,
            "verifier_lock_sha256":hash(include_bytes!("../Cargo.lock")),
            "wire_sha256":hash(wire),
            "request_sha256":hash(request_bytes)
        })
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn limits() -> Result<(), Box<dyn std::error::Error>> {
    // One-shot parsing/verification only, with fixed lower-than-driver limits.
    // No permission to raise either limit if a future payload does not fit.
    for (resource, limit) in [
        (libc::RLIMIT_AS, 1_073_741_824),
        (libc::RLIMIT_CPU, 2),
        (libc::RLIMIT_FSIZE, 4096),
    ] {
        let value = libc::rlimit {
            rlim_cur: limit,
            rlim_max: limit,
        };
        // SAFETY: a valid rlimit pointer, affecting only this one-shot process.
        if unsafe { libc::setrlimit(resource, &value) } != 0 {
            return Err("verifier resource limits unavailable".into());
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn limits() -> Result<(), Box<dyn std::error::Error>> {
    Err("bounded verifier requires Linux".into())
}

fn main() {
    if verify().is_err() {
        // Never leak receipt contents or source paths through diagnostics.
        eprintln!("acquisition verification refused");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_preserves_outer_whitespace_and_refuses_extra_or_short_data() {
        let request = b"{}";
        let wire = b" \n{ }\t";
        let mut frame = serde_json::to_vec(&json!({"schema":"seeds-acquisition-input/v1",
            "request_bytes":request.len(),"wire_bytes":wire.len()}))
        .unwrap();
        frame.push(b'\n');
        frame.extend_from_slice(request);
        frame.extend_from_slice(wire);
        let decoded = unframe(&frame).unwrap();
        assert_eq!(decoded.request, request);
        assert_eq!(decoded.wire, wire);
        assert!(unframe(&frame[..frame.len() - 1]).is_err());
        frame.push(b'x');
        assert!(unframe(&frame).is_err());
    }
}
