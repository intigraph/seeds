//! Explicit command-scoped credential input. No provider, shell or environment
//! token lookup is performed here; the caller admits the destination first.
use crate::error::{ErrorKind, Result, SdError};

const CAP: usize = 4096;

fn unavailable() -> SdError {
    SdError::new(
        ErrorKind::Config,
        "selected credential unavailable or invalid",
    )
}

/// An explicitly selected identity and an inherited anonymous private file.
/// The principal is a declaration, never proof of server-authenticated identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    principal: String,
    fd: i32,
}

impl Selected {
    /// Both fields must be present. Descriptor metadata contains no bearer.
    pub fn parse(principal: Option<&str>, fd: Option<&str>) -> Result<Option<Self>> {
        match (principal, fd) {
            (None, None) => Ok(None),
            (Some(principal), Some(fd))
                if !principal.is_empty()
                    && principal.len() <= 128
                    && principal
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.:@".contains(&b)) =>
            {
                let fd = fd.parse::<i32>().map_err(|_| unavailable())?;
                if fd < 3 {
                    return Err(unavailable());
                }
                Ok(Some(Self {
                    principal: principal.into(),
                    fd,
                }))
            }
            _ => Err(unavailable()),
        }
    }

    /// Read at most CAP bytes from a duplicated descriptor. Seek is positional:
    /// neither this backend nor another backend advances the provider's cursor.
    /// The duplicate closes on all paths; the launcher owns the inherited fd.
    #[cfg(unix)]
    pub fn read(&self) -> Result<String> {
        use std::fs::File;
        use std::os::fd::FromRawFd;
        use std::os::unix::fs::{FileExt, MetadataExt};
        // SAFETY: fcntl validates the supplied number and returns a newly owned
        // descriptor. File takes only that duplicate, never a caller-owned fd.
        let duplicate = unsafe { libc::fcntl(self.fd, libc::F_DUPFD_CLOEXEC, 3) };
        if duplicate < 0 {
            return Err(unavailable());
        }
        let file = unsafe { File::from_raw_fd(duplicate) };
        let meta = file.metadata().map_err(|_| unavailable())?;
        // Only an unlinked regular file / anonymous memfd, owned by this uid,
        // private, and bounded. Pipes/devices cannot block a credential read.
        if !meta.is_file()
            || meta.nlink() != 0
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o077 != 0
            || meta.len() == 0
            || meta.len() > CAP as u64
        {
            return Err(unavailable());
        }
        let mut bytes = vec![0; CAP + 1];
        let count = file.read_at(&mut bytes, 0).map_err(|_| unavailable())?;
        if count != meta.len() as usize {
            return Err(unavailable());
        }
        bytes.truncate(count);
        if bytes.is_empty() || bytes.len() > CAP || bytes.iter().any(|b| !b.is_ascii_graphic()) {
            return Err(unavailable());
        }
        String::from_utf8(bytes).map_err(|_| unavailable())
    }

    #[cfg(not(unix))]
    pub fn read(&self) -> Result<String> {
        Err(unavailable())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn input(bytes: &[u8], linked: bool) -> (File, std::path::PathBuf) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "seeds-credential-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&p)
            .unwrap();
        f.write_all(bytes).unwrap();
        if !linked {
            std::fs::remove_file(&p).unwrap();
        }
        (f, p)
    }

    fn selection(f: &File) -> Selected {
        Selected::parse(Some("test-principal"), Some(&f.as_raw_fd().to_string()))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn anonymous_private_descriptor_and_rotation() {
        let (f, _) = input(b"own-test-value", false);
        assert_eq!(selection(&f).read().unwrap(), "own-test-value");
        assert_eq!(selection(&f).read().unwrap(), "own-test-value");
        let (new, _) = input(b"rotated-test-value", false);
        assert_eq!(selection(&new).read().unwrap(), "rotated-test-value");
    }

    #[test]
    fn linked_or_public_descriptor_is_refused() {
        let (f, p) = input(b"fixture", true);
        assert!(selection(&f).read().is_err());
        std::fs::remove_file(p).unwrap();
        f.set_permissions(std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(selection(&f).read().is_err());
    }

    #[test]
    fn malformed_oversize_and_closed_are_refused_without_echo() {
        for bytes in [
            b"".as_slice(),
            b"line\nbreak",
            b"\0",
            &[0xff],
            &vec![b'x'; CAP + 1],
        ] {
            let (f, _) = input(bytes, false);
            let err = selection(&f).read().unwrap_err().to_string();
            assert!(err.contains("selected credential unavailable or invalid"));
        }
        assert!(Selected::parse(Some("test"), Some("2147483647"))
            .unwrap()
            .unwrap()
            .read()
            .is_err());
        for pair in [
            (Some("test"), None),
            (None, Some("3")),
            (Some("test"), Some("2")),
            (Some("bad\nname"), Some("3")),
        ] {
            assert!(Selected::parse(pair.0, pair.1).is_err());
        }
        assert!(Selected::parse(None, None).unwrap().is_none());
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let selected = Selected::parse(Some("test"), Some(&socket.as_raw_fd().to_string()))
            .unwrap()
            .unwrap();
        assert!(selected.read().is_err());
    }
}
