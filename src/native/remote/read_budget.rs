//! Cooperative host admission shared with shantytown's read-budget-v1 ledger.
//! Default OFF. Charge every RemoteBackend request conservatively, including
//! bootstrap and writes; no permission or write authority is granted here.
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn refused(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("local Quipu read budget: {message}; no request dispatched"),
    )
}

pub(super) fn limit() -> io::Result<Option<usize>> {
    let raw = std::env::var("ST_QUIPU_READS_PER_MINUTE").unwrap_or_else(|_| "0".into());
    let value = raw
        .trim()
        .parse::<usize>()
        .map_err(|_| refused("invalid limit"))?;
    if value > 10000 {
        return Err(refused("invalid limit"));
    }
    Ok((value != 0).then_some(value))
}

fn admit(request_url: &str) -> io::Result<()> {
    let Some(limit) = limit()? else {
        return Ok(());
    };
    let url = url::Url::parse(request_url).map_err(|_| refused("invalid origin"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(refused("invalid origin"));
    }
    // Match Python json.dumps([scheme, hostname, effective_port]) byte-for-byte.
    // Non-ASCII/IPv6 spellings need a separate cross-language canonical contract;
    // refuse them while enabled rather than silently allocate a second ledger.
    let host = url.host_str().ok_or_else(|| refused("missing host"))?;
    let authority = request_url
        .split_once("://")
        .ok_or_else(|| refused("invalid origin"))?
        .1
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    let raw_host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _)| host);
    if raw_host.to_lowercase() != host.to_lowercase()
        || request_url.chars().any(|c| c.is_ascii_control())
    {
        return Err(refused("noncanonical origin spelling"));
    }
    if !host.is_ascii() || host.contains(':') || request_url.chars().any(|c| !c.is_ascii()) {
        return Err(refused("unsupported origin spelling"));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| refused("missing port"))?;
    let origin = format!(
        "[{}, {}, {}]",
        serde_json::to_string(url.scheme())?,
        serde_json::to_string(&host.to_lowercase())?,
        port
    );
    let key = format!("{:x}", Sha256::digest(origin.as_bytes()));
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".cache")))
        .ok_or_else(|| refused("cache home unavailable"))?;
    let directory = base.join("shantytown/read-budget-v1");
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(&directory)?;
    let metadata = fs::symlink_metadata(&directory)?;
    // SAFETY: getuid has no preconditions or memory access.
    let uid = unsafe { libc::getuid() };
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(refused("unsafe ledger directory"));
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(key + ".json"))?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(refused("unsafe ledger file"));
    }
    let deadline = Instant::now() + Duration::from_millis(100);
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err(refused("ledger busy or unavailable")),
        }
    }
    let mut bytes = Vec::new();
    (&mut file).take(256001).read_to_end(&mut bytes)?;
    let mut times: Vec<f64> = if bytes.is_empty() {
        Vec::new()
    } else {
        serde_json::from_slice(&bytes).map_err(|_| refused("invalid ledger"))?
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| refused("invalid clock"))?
        .as_secs_f64();
    if bytes.len() > 256000
        || times.len() > 10000
        || times.iter().any(|t| !t.is_finite() || *t < 0.0 || *t > now)
        || times.windows(2).any(|p| p[0] > p[1])
    {
        return Err(refused("invalid ledger"));
    }
    times.retain(|t| now - *t < 60.0);
    if times.len() >= limit {
        return Err(refused("exhausted; retry after the rolling minute"));
    }
    times.push(now);
    let body = serde_json::to_vec(&times)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&body)?;
    file.set_len(body.len() as u64)?;
    // Drop the lock before network I/O. Failed requests keep their charge.
    Ok(())
}

pub(super) fn middleware(
    request: ureq::Request,
    next: ureq::MiddlewareNext<'_>,
) -> Result<ureq::Response, ureq::Error> {
    admit(request.url()).map_err(|e| refused(&e.to_string()))?;
    next.handle(request)
}
