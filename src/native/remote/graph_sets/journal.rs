//! Private, bounded client custody. Publication precedes the first send.
use super::{invalid, Attempt};
use crate::error::{ErrorKind, Result, SdError};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const MAX_ENVELOPES: usize = 128;
const MAX_ENVELOPE_BYTES: u64 = 4 * 1024 * 1024;

pub(super) struct Journal {
    root: PathBuf,
    dir: File,
    _lock: File,
}

fn private(meta: &fs::Metadata, directory: bool) -> bool {
    // SAFETY: geteuid has no arguments or ownership requirements.
    let uid = unsafe { libc::geteuid() };
    meta.uid() == uid
        && meta.mode() & 0o077 == 0
        && if directory {
            meta.is_dir()
        } else {
            meta.is_file() && meta.nlink() == 1
        }
}

pub(super) fn read_private(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| invalid("private file is missing, unreadable or a symlink"))?;
    let meta = file
        .metadata()
        .map_err(|_| invalid("private file metadata"))?;
    if !private(&meta, false) || meta.len() > limit {
        return Err(invalid("private file owner, mode, links or size"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("private file read"))?;
    if bytes.len() as u64 > limit {
        return Err(invalid("private file grew beyond bound"));
    }
    Ok(bytes)
}

impl Journal {
    pub(super) fn open(root: PathBuf) -> Result<Self> {
        if !root.is_absolute() {
            return Err(invalid("attempt directory must be absolute"));
        }
        if !root.exists() {
            fs::create_dir_all(&root).map_err(|_| invalid("attempt directory creation"))?;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                .map_err(|_| invalid("attempt directory permissions"))?;
        }
        let dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(&root)
            .map_err(|_| invalid("attempt directory must not be a symlink"))?;
        if !private(
            &dir.metadata()
                .map_err(|_| invalid("attempt directory metadata"))?,
            true,
        ) {
            return Err(invalid(
                "attempt directory must be private and owned by this user",
            ));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(root.join(".lock"))
            .map_err(|_| invalid("attempt lock open"))?;
        if !private(
            &lock
                .metadata()
                .map_err(|_| invalid("attempt lock metadata"))?,
            false,
        ) {
            return Err(invalid("attempt lock owner or mode"));
        }
        // SAFETY: lock owns a live descriptor. The nonblocking lock lasts to Drop.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(invalid("another invocation owns the attempt journal"));
        }
        let journal = Self {
            root,
            dir,
            _lock: lock,
        };
        journal.check_root()?;
        Ok(journal)
    }
    fn check_root(&self) -> Result<()> {
        let path = fs::symlink_metadata(&self.root)
            .map_err(|_| invalid("attempt directory disappeared"))?;
        let pinned = self
            .dir
            .metadata()
            .map_err(|_| invalid("attempt directory descriptor"))?;
        if !private(&path, true) || path.dev() != pinned.dev() || path.ino() != pinned.ino() {
            return Err(invalid("attempt directory identity changed"));
        }
        Ok(())
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        if !super::hex_id(id) {
            return Err(invalid("invalid write ID"));
        }
        self.check_root()?;
        Ok(self.root.join(format!("{id}.json")))
    }
    pub(super) fn pending(&self) -> Result<Vec<Attempt>> {
        self.check_root()?;
        let mut out = Vec::new();
        let entries = fs::read_dir(&self.root).map_err(|_| invalid("attempt directory listing"))?;
        for (count, entry) in entries.enumerate() {
            if count > MAX_ENVELOPES * 2 {
                return Err(invalid("attempt directory inventory bound"));
            }
            let entry = entry.map_err(|_| invalid("attempt entry"))?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| invalid("attempt file name"))?;
            if name == ".lock" {
                continue;
            }
            // Interrupted staging files have never been sent; retain, never execute them.
            if name.starts_with(".stage-") {
                continue;
            }
            let id = name
                .strip_suffix(".json")
                .filter(|id| super::hex_id(id))
                .ok_or_else(|| invalid("unknown file in attempt directory"))?;
            let bytes = read_private(&self.path(id)?, MAX_ENVELOPE_BYTES)?;
            let attempt: Attempt =
                serde_json::from_slice(&bytes).map_err(|_| invalid("corrupt attempt envelope"))?;
            attempt.validate()?;
            if attempt.write_id != id {
                return Err(invalid("attempt file and ID differ"));
            }
            out.push(attempt);
            if out.len() > MAX_ENVELOPES {
                return Err(invalid("pending attempt capacity"));
            }
        }
        self.check_root()?;
        out.sort_by(|a, b| a.write_id.cmp(&b.write_id));
        Ok(out)
    }
    pub(super) fn persist(&self, attempt: &Attempt, fresh: bool) -> Result<()> {
        attempt.validate()?;
        let destination = self.path(&attempt.write_id)?;
        if fresh && self.pending()?.len() >= MAX_ENVELOPES {
            return Err(invalid("pending attempt capacity"));
        }
        let bytes = serde_json::to_vec(attempt).map_err(|_| invalid("attempt serialization"))?;
        if bytes.len() as u64 > MAX_ENVELOPE_BYTES {
            return Err(invalid("attempt envelope size"));
        }
        let stage = self.root.join(format!(".stage-{}", super::new_id()?));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&stage)
            .map_err(|_| invalid("attempt staging"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| invalid("attempt fsync"))?;
        self.check_root()?;
        let publication = if fresh {
            fs::hard_link(&stage, &destination)
        } else {
            fs::rename(&stage, &destination)
        };
        if publication.is_err() {
            let _ = fs::remove_file(stage);
            return Err(invalid("attempt publish refused"));
        }
        if fresh {
            fs::remove_file(&stage).map_err(|_| invalid("attempt stage unlink"))?;
        }
        self.dir.sync_all().map_err(|_| {
            SdError::new(
                ErrorKind::Indeterminate,
                "attempt published but directory durability UNKNOWN; no send",
            )
        })?;
        Ok(())
    }
    pub(super) fn remove_terminal(&self, attempt: &Attempt) -> Result<()> {
        if !matches!(attempt.phase.as_str(), "finished" | "sealed") {
            return Err(invalid(
                "only verified finished/sealed attempts may be removed",
            ));
        }
        fs::remove_file(self.path(&attempt.write_id)?)
            .map_err(|_| invalid("terminal attempt unlink"))?;
        self.dir.sync_all().map_err(|_| {
            SdError::new(
                ErrorKind::Indeterminate,
                "terminal resolution known, directory durability UNKNOWN",
            )
        })
    }
}
