//! One resident shared-applier consumer; short-lived CLI reads use private IPC.
//! Operator admission/bootstrap is explicit. There is no polling read fallback.
use super::{
    cli::{Cli, Command},
    config::{Location, Resolved},
    stream_projection::StreamBoard,
    Outcome,
};
use crate::{
    backend::{Claims, Ctx},
    error::{Result, SdError},
    projection::SourceBinding,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime},
};

mod query_scope;
mod transport;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryArgs {
    query: String,
}

const MAX_REQUEST: usize = 64 * 1024;
const MAX_REPLY: usize = 16 * 1024 * 1024;
const MAX_FRESH_MS: u64 = 5000;
const IO_TIMEOUT: Duration = Duration::from_secs(2);

fn unknown(reason: &str) -> SdError {
    SdError::refused(format!(
        "push read UNKNOWN: {reason}; no primary polling fallback"
    ))
}
fn io<T>(result: std::io::Result<T>) -> Result<T> {
    result.map_err(|_| unknown("local transport or pinned input unavailable"))
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| unknown("invalid bounded protocol input"))
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| unknown("protocol encoding failed"))
}
fn uid() -> u32 {
    // SAFETY: getter only; no identity or privilege operation.
    unsafe { libc::geteuid() }
}
fn private_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = io(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path))?;
    let meta = io(file.metadata())?;
    if !meta.is_file() || meta.uid() != uid() || meta.mode() & 0o077 != 0 || meta.len() > limit {
        return Err(unknown(
            "input must be bounded, private and owned by this UID",
        ));
    }
    let mut bytes = Vec::new();
    io(file.take(limit + 1).read_to_end(&mut bytes))?;
    if bytes.len() as u64 > limit {
        return Err(unknown("input grew beyond its bound"));
    }
    Ok(bytes)
}
fn socket_parent(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(unknown("absolute local socket required"));
    }
    let meta = io(fs::symlink_metadata(
        path.parent()
            .ok_or_else(|| unknown("socket parent missing"))?,
    ))?;
    if !meta.is_dir() || meta.uid() != uid() || meta.mode() & 0o077 != 0 {
        return Err(unknown("socket parent must be private to this UID"));
    }
    Ok(())
}

/// An operator-supplied packet, never discovered from a stream response.
/// Contains no secret. Manifest hash and signing public key are trust inputs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub primary_url: String,
    pub graph: String,
    pub principal: String,
    pub replication_credential_file: PathBuf,
    pub store_id: String,
    pub log_epoch: String,
    pub scope_digest: String,
    pub policy_digest: String,
    pub stream_key: String,
    pub manifest: PathBuf,
    pub manifest_sha256: String,
    pub bootstrap: PathBuf,
    pub socket: PathBuf,
    pub max_freshness_ms: u64,
}
impl Config {
    pub fn load(path: &Path, cfg: &Resolved) -> Result<Self> {
        let c: Self = decode(&private_file(path, MAX_REQUEST as u64)?)?;
        if cfg.location != Location::Url(c.primary_url.clone())
            || cfg.graph() != c.graph
            || c.principal.is_empty()
            || c.principal.len() > 128
            || [&c.store_id, &c.log_epoch, &c.scope_digest, &c.policy_digest]
                .iter()
                .any(|s| s.is_empty())
            || !(1..=MAX_FRESH_MS).contains(&c.max_freshness_ms)
            || c.stream_key.len() != 64
            || !c.stream_key.bytes().all(|b| b.is_ascii_hexdigit())
            || c.manifest_sha256.len() != 64
            || !c.manifest_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !c.replication_credential_file.is_absolute()
            || !c.manifest.is_absolute()
            || !c.bootstrap.is_absolute()
        {
            return Err(unknown(
                "authoritative URL/graph or operator binding mismatch",
            ));
        }
        socket_parent(&c.socket)?;
        Ok(c)
    }
    fn binding(&self) -> SourceBinding {
        SourceBinding {
            store_id: self.store_id.clone(),
            log_epoch: self.log_epoch.clone(),
            scope_digest: self.scope_digest.clone(),
            policy_digest: self.policy_digest.clone(),
        }
    }
    fn bootstrap(&self) -> Result<StreamBoard> {
        let bytes = private_file(&self.manifest, 1024 * 1024)?;
        if super::attest::sha256_hex(&bytes) != self.manifest_sha256 {
            return Err(unknown("bootstrap manifest differs from its operator pin"));
        }
        let manifest = decode(&bytes)?;
        let mut board =
            StreamBoard::empty(&self.graph, manifest, &self.binding(), &self.stream_key)?;
        // Bounded regular input only; no network cut, symlink, FIFO or import.
        let bytes = private_file(&self.bootstrap, 256 * 1024 * 1024)?;
        let mut frames = Frames::new(BufReader::new(bytes.as_slice()));
        while let Some((event, data)) = frames.next_frame()? {
            board.apply_frame(&event, &data, 0)?;
        }
        board.stream_request()?; // incomplete/aborted bootstrap is never empty success
        Ok(board)
    }
}

/// Only these existing board operations can cross IPC. No raw shell/argv or SQL.
pub fn eligible(command: &Command) -> bool {
    matches!(
        command,
        Command::List(_) | Command::Ready(_) | Command::Blocked(_) | Command::Show(_)
    )
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    primary_url: String,
    graph: String,
    principal: String,
    command: String,
    args: Value,
    json: bool,
    format: Option<String>,
    actor: String,
    now: String,
    max_ms: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    version: u32,
    primary_url: String,
    graph: String,
    principal: String,
    store_id: String,
    log_epoch: String,
    scope_digest: String,
    policy_digest: String,
    source_tx: u64,
    local_tx: u64,
    bound_ms: u64,
    result: Outcome,
}
fn send<T: Serialize>(stream: &mut UnixStream, value: &T, limit: usize) -> Result<()> {
    let data = encode(value)?;
    if data.len() > limit {
        return Err(unknown("IPC output exceeds its bound"));
    }
    io(stream.write_all(&(data.len() as u32).to_be_bytes()))?;
    io(stream.write_all(&data))
}
fn receive<T: serde::de::DeserializeOwned>(stream: &mut UnixStream, limit: usize) -> Result<T> {
    let mut header = [0; 4];
    io(stream.read_exact(&mut header))?;
    let count = u32::from_be_bytes(header) as usize;
    if count == 0 || count > limit {
        return Err(unknown("IPC frame exceeds its bound"));
    }
    let mut bytes = vec![0; count];
    io(stream.read_exact(&mut bytes))?;
    decode(&bytes)
}
fn timeout(stream: &UnixStream) -> Result<()> {
    io(stream.set_read_timeout(Some(IO_TIMEOUT)))?;
    io(stream.set_write_timeout(Some(IO_TIMEOUT)))
}

pub fn read(cli: &Cli, cfg: &Resolved, ctx: &Ctx, path: &Path) -> Result<Outcome> {
    let c = Config::load(path, cfg)?;
    let args = match &cli.command {
        Command::List(a) => serde_json::to_value(a),
        Command::Ready(a) => serde_json::to_value(a),
        Command::Show(a) => serde_json::to_value(a),
        Command::Blocked(a) => serde_json::to_value(a),
        Command::ReadStreamQuery => {
            let mut bytes = Vec::new();
            io(std::io::stdin().take(60 * 1024 + 1).read_to_end(&mut bytes))?;
            if bytes.len() > 60 * 1024 {
                return Err(unknown("query input exceeds budget"));
            }
            let args: QueryArgs = decode(&bytes)?;
            serde_json::to_value(args)
        }
        _ => return Err(unknown("operation is not a board read")),
    }
    .map_err(|_| unknown("read argument encoding failed"))?;
    let meta = io(fs::symlink_metadata(&c.socket))?;
    if !meta.file_type().is_socket() || meta.uid() != uid() || meta.mode() & 0o077 != 0 {
        return Err(unknown("socket is not private to its admitted UID"));
    }
    let started = Instant::now();
    let mut stream = io(UnixStream::connect(&c.socket))?;
    timeout(&stream)?;
    send(
        &mut stream,
        &Request {
            version: 1,
            primary_url: c.primary_url.clone(),
            graph: c.graph.clone(),
            principal: c.principal.clone(),
            command: cli.command.name().into(),
            args,
            json: cli.json,
            format: cli.format.clone(),
            actor: ctx.actor.clone(),
            now: ctx.now.clone(),
            max_ms: c.max_freshness_ms,
        },
        MAX_REQUEST,
    )?;
    let reply: std::result::Result<Reply, String> = receive(&mut stream, MAX_REPLY)?;
    let mut reply = reply.map_err(|_| unknown("consumer refused freshness or read binding"))?;
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if reply.version != 1
        || reply.primary_url != c.primary_url
        || reply.graph != c.graph
        || reply.principal != c.principal
        || reply.store_id != c.store_id
        || reply.log_epoch != c.log_epoch
        || reply.scope_digest != c.scope_digest
        || reply.policy_digest != c.policy_digest
        || reply.bound_ms.saturating_add(elapsed) > c.max_freshness_ms
    {
        return Err(unknown("IPC binding or end-to-end freshness mismatch"));
    }
    let notice = format!(
        "sd: push read source tx {}, local tx {}, freshness bound <= {} ms",
        reply.source_tx,
        reply.local_tx,
        reply.bound_ms.saturating_add(elapsed)
    );
    if !reply.result.stderr.is_empty() {
        reply.result.stderr.push('\n');
    }
    reply.result.stderr.push_str(&notice);
    Ok(reply.result)
}

fn render(
    board: &mut StreamBoard,
    c: &Config,
    cfg: &Resolved,
    request: Request,
    now: u64,
) -> Result<Reply> {
    if request.version != 1
        || request.primary_url != c.primary_url
        || request.graph != c.graph
        || request.principal != c.principal
        || request.max_ms == 0
        || request.max_ms > c.max_freshness_ms
        || request.actor.len() > 2048
        || request.now.len() > 64
    {
        return Err(unknown("read request differs from admitted binding"));
    }
    if request.command == "read-stream-query" {
        let args: QueryArgs = decode(&encode(&request.args)?)?;
        let (value, p, bound) = board.read(now, request.max_ms, |backend, tx| {
            query_scope::evaluate(backend, &c.graph, tx, &args.query)
        })?;
        return Ok(Reply {
            version: 1,
            primary_url: c.primary_url.clone(),
            graph: c.graph.clone(),
            principal: c.principal.clone(),
            store_id: p.source.store_id,
            log_epoch: p.source.log_epoch,
            scope_digest: p.source.scope_digest,
            policy_digest: p.source.policy_digest,
            source_tx: p.source_tx,
            local_tx: p.local_tx,
            bound_ms: bound,
            result: Outcome {
                stdout: serde_json::to_string(&value)
                    .map_err(|_| unknown("query encoding failed"))?,
                stderr: String::new(),
                code: 0,
            },
        });
    }
    let mut cli =
        Cli::try_parse_from(["sd", "list"]).map_err(|_| unknown("read parser unavailable"))?;
    cli.command = match request.command.as_str() {
        "list" => Command::List(decode(&encode(&request.args)?)?),
        "ready" => Command::Ready(decode(&encode(&request.args)?)?),
        "show" => Command::Show(decode(&encode(&request.args)?)?),
        "blocked" => Command::Blocked(decode(&encode(&request.args)?)?),
        _ => return Err(unknown("IPC cannot execute a mutation or another command")),
    };
    cli.json = request.json;
    cli.format = request.format;
    // Parse was of a safe command; still validate format independently of a client.
    if cli
        .format
        .as_deref()
        .is_some_and(|f| !matches!(f, "text" | "json" | "csv"))
    {
        return Err(unknown("unsupported output format"));
    }
    super::csv_usage(&cli)?;
    let ctx = Ctx {
        now: request.now,
        actor: request.actor,
        prefix: cfg.prefix.clone(),
        claims: Claims::default(),
    };
    let (result, p, bound) = board.read(now, request.max_ms, |backend, local_tx| {
        cli.at = Some(local_tx);
        match super::dispatch(&cli, cfg, &ctx, backend) {
            Ok(outcome) => Ok(outcome),
            // Fresh, complete coverage can prove a caller's missing ID or bad
            // selector. Preserve the ordinary CLI contract; projection/store
            // failures still escape as UNKNOWN rather than domain answers.
            Err(error)
                if matches!(
                    error.kind,
                    crate::error::ErrorKind::NotFound | crate::error::ErrorKind::Usage
                ) =>
            {
                Ok(super::error_outcome(cli.wants_json(), &error, vec![]))
            }
            Err(error) => Err(error),
        }
    })?;
    Ok(Reply {
        version: 1,
        primary_url: c.primary_url.clone(),
        graph: c.graph.clone(),
        principal: c.principal.clone(),
        store_id: p.source.store_id,
        log_epoch: p.source.log_epoch,
        scope_digest: p.source.scope_digest,
        policy_digest: p.source.policy_digest,
        source_tx: p.source_tx,
        local_tx: p.local_tx,
        bound_ms: bound,
        result,
    })
}

/// Bounded SSE envelope reader only; all payload/transaction/signature semantics
/// belong to the shared applier. Partial final events and unknown fields refuse.
struct Frames<R> {
    reader: R,
}
impl<R: BufRead> Frames<R> {
    fn new(reader: R) -> Self {
        Self { reader }
    }
    fn next_frame(&mut self) -> Result<Option<(String, String)>> {
        let (mut event, mut data) = (String::new(), String::new());
        loop {
            let mut raw = Vec::new();
            let count = io(self
                .reader
                .by_ref()
                .take((quipu::feed_parts::MAX_FRAME_BYTES + 1) as u64)
                .read_until(b'\n', &mut raw))?;
            if count == 0 {
                return if event.is_empty() && data.is_empty() {
                    Ok(None)
                } else {
                    Err(unknown("truncated SSE frame"))
                };
            }
            if raw.len() > quipu::feed_parts::MAX_FRAME_BYTES {
                return Err(unknown("SSE line exceeds budget"));
            }
            let line = std::str::from_utf8(&raw)
                .map_err(|_| unknown("SSE is not UTF-8"))?
                .trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if event.is_empty() && data.is_empty() {
                    continue;
                }
                if event.is_empty() || data.is_empty() {
                    return Err(unknown("incomplete SSE envelope"));
                }
                data.pop();
                return Ok(Some((event, data)));
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line
                .split_once(':')
                .ok_or_else(|| unknown("malformed SSE envelope"))?;
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" if event.is_empty() && value.len() <= 128 => event = value.into(),
                "data" => {
                    data.push_str(value);
                    data.push('\n');
                }
                _ => return Err(unknown("unexpected SSE envelope field")),
            }
            if event.len() + data.len() > quipu::feed_parts::MAX_FRAME_BYTES {
                return Err(unknown("SSE frame exceeds budget"));
            }
        }
    }
}

/// Clock discontinuities (including suspend or wall-clock changes) discard
/// proofs. Instant alone can stop during suspend on some supported hosts.
struct Clock {
    origin: Instant,
    last: (Duration, SystemTime),
}
impl Clock {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            last: (Duration::ZERO, SystemTime::now()),
        }
    }
    fn sample(&mut self) -> (u64, bool) {
        let elapsed = self.origin.elapsed();
        let wall = SystemTime::now();
        let monotonic = elapsed.saturating_sub(self.last.0);
        let changed = wall
            .duration_since(self.last.1)
            .map_or(true, |w| w.abs_diff(monotonic) > Duration::from_secs(1));
        self.last = (elapsed, wall);
        (
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            changed,
        )
    }
}

pub fn run(cfg: &Resolved, path: &Path, check: bool) -> Result<Outcome> {
    let c = Config::load(path, cfg)?;
    let mut board = c.bootstrap()?;
    if check {
        return Ok(super::ok(
            true,
            json!({"checked":true,"network":false,"socket":false,"readiness":"not activated"}),
            "pinned bootstrap packet valid".into(),
            vec![],
        ));
    }
    // Dedicated replication credential only; no writer key, ordinary bearer,
    // /identity request, enrollment or fallback. Principal/source admission is
    // supplied by the reviewed pinned operator packet, not inferred here.
    let remote = transport::Transport::new(cfg, &c)?;
    let tail = board.stream_request()?;
    let body = encode(
        &json!({"graphs":tail.graphs,"context_graphs":tail.context_graphs,"since":tail.since,"expected_store_id":tail.expected_store_id}),
    )?;
    let request = remote.request("/replicate/stream")?;
    let listener = io(UnixListener::bind(&c.socket))?; // never unlink/adopt an existing socket
    io(fs::set_permissions(
        &c.socket,
        fs::Permissions::from_mode(0o600),
    ))?;
    io(listener.set_nonblocking(true))?;
    // The socket is deliberately retained on failure, requiring explicit normal
    // service recovery rather than silently replacing another consumer.
    let (frames, incoming) = mpsc::sync_channel(64);
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let response = request
                .send_bytes(&body)
                .map_err(|_| unknown("replication stream refused or disconnected"))?;
            if response
                .header("Content-Type")
                .is_none_or(|t| !t.starts_with("text/event-stream"))
            {
                return Err(unknown("stream content type mismatch"));
            }
            let mut parser = Frames::new(BufReader::new(response.into_reader()));
            while let Some(frame) = parser.next_frame()? {
                if frames.send(Ok(frame)).is_err() {
                    return Ok(());
                }
            }
            Err(unknown("stream ended"))
        })();
        if let Err(error) = result {
            let _ = frames.send(Err(error));
        }
    });
    let (challenges, challenge_requests) = mpsc::sync_channel::<(ureq::Request, Vec<u8>)>(1);
    let (replies, challenge_replies) = mpsc::sync_channel(1);
    thread::spawn(move || {
        while let Ok((request, body)) = challenge_requests.recv() {
            let ok = request.send_bytes(&body).is_ok_and(|r| r.status() == 202);
            if replies.send(ok).is_err() {
                break;
            }
        }
    });
    let mut clock = Clock::new();
    let mut next_challenge = 0;
    let mut connected = true;
    loop {
        let (now, changed) = clock.sample();
        if changed {
            board.clock_discontinuity();
        }
        for _ in 0..64 {
            match incoming.try_recv() {
                Ok(Ok((event, data))) if connected => {
                    if board.apply_frame(&event, &data, now).is_err() {
                        board.disconnect();
                        connected = false;
                    }
                }
                Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                    board.disconnect();
                    connected = false;
                    break;
                }
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if challenge_replies.try_recv().is_ok_and(|ok| !ok) {
            board.disconnect();
            connected = false;
        }
        if connected && now >= next_challenge {
            let mut nonce = [0; 16];
            getrandom::getrandom(&mut nonce)
                .map_err(|_| unknown("challenge entropy unavailable"))?;
            if let Ok(session) = board.challenge(nonce, now) {
                let nonce: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
                let body = encode(&json!({"session":session,"nonce":nonce}))?;
                let request = remote.request("/replicate/challenge")?;
                if challenges.try_send((request, body)).is_err() {
                    board.disconnect();
                    connected = false;
                }
            }
            next_challenge = now.saturating_add(2000);
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let result = timeout(&stream)
                    .and_then(|_| receive::<Request>(&mut stream, MAX_REQUEST))
                    .and_then(|r| {
                        let (now, changed) = clock.sample();
                        if changed {
                            board.clock_discontinuity();
                        }
                        let budget = r.max_ms;
                        render(&mut board, &c, cfg, r, now).map(|reply| (reply, budget))
                    });
                let (after, changed) = clock.sample();
                if changed {
                    board.clock_discontinuity();
                }
                let result = result
                    .and_then(|(mut r, budget)| {
                        // charge rendering time, never certify an old capture
                        r.bound_ms = r.bound_ms.saturating_add(after.saturating_sub(now));
                        if changed || r.bound_ms > budget {
                            return Err(unknown("read exceeded proof budget"));
                        }
                        Ok(r)
                    })
                    .map_err(|_| "push read UNKNOWN; no primary polling fallback".to_string());
                let _ = send(&mut stream, &result, MAX_REPLY);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(_) => return Err(unknown("local listener failed")),
        }
    }
}

#[cfg(test)]
mod tests;
