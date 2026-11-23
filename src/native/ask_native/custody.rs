//! A reservation survives failed sends and deployment/code changes.
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use super::profile::{absolute, refused};
use crate::error::{Result, SdError};
use serde_json::Value;

pub(super) fn private_file(path: &Path, uid: u32) -> Result<String> {
    absolute(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| refused())?;
    let m = file.metadata().map_err(|_| refused())?;
    if !m.is_file() || m.uid() != uid || m.permissions().mode() & 0o077 != 0 {
        return Err(refused());
    }
    let mut value = String::new();
    file.take(4097)
        .read_to_string(&mut value)
        .map_err(|_| refused())?;
    let value = value.trim();
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(refused());
    }
    Ok(value.to_string())
}

pub(super) fn reserve(path: &Path, uid: u32, identity: &Value, record: &Value) -> Result<()> {
    absolute(path)?;
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| refused())?;
    let m = dir.metadata().map_err(|_| refused())?;
    // SAFETY: geteuid has no arguments or ownership effects.
    if uid == 0
        || uid != unsafe { libc::geteuid() }
        || m.uid() != uid
        || m.permissions().mode() & 0o777 != 0o700
    {
        return Err(refused());
    }
    let name = format!(
        "{}.json",
        crate::native::attest::sha256_hex(identity.to_string().as_bytes())
    );
    let name = std::ffi::CString::new(name).map_err(|_| refused())?;
    // SAFETY: the borrowed live directory fd and NUL-terminated leaf stay valid;
    // ownership of a successful new fd transfers immediately to File below.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(SdError::refused(
            "Ask send already reserved or custody unavailable; retain intent and reconcile only",
        ));
    }
    // SAFETY: openat just returned this uniquely owned file descriptor.
    let mut file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata().map_err(|_| refused())?;
    if metadata.uid() != uid || metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(refused());
    }
    // Never remove/release a reservation, even on an fsync/write failure.
    file.write_all(record.to_string().as_bytes())
        .map_err(|_| refused())?;
    file.sync_all().map_err(|_| refused())?;
    dir.sync_all().map_err(|_| refused())?;
    Ok(())
}
