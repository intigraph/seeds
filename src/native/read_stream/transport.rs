//! Dedicated Replicate-only credential, separate from ordinary reads/writes.
use super::*;

pub(super) struct Transport {
    base: String,
    credential: PathBuf,
    agent: ureq::Agent,
}
impl Transport {
    pub(super) fn new(cfg: &Resolved, c: &Config) -> Result<Self> {
        let url =
            url::Url::parse(&c.primary_url).map_err(|_| unknown("invalid replication URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || c.primary_url.chars().any(char::is_whitespace)
            || !cfg.token_allowed(&c.primary_url, cfg.location_from_project)
            || (!super::super::remote::secure_enough(&c.primary_url)
                && !super::super::remote::plain_http_allowed(
                    &c.primary_url,
                    &cfg.allow_plain_http_hosts,
                ))
        {
            return Err(unknown("replication destination is not admitted"));
        }
        Ok(Self {
            base: c.primary_url.trim_end_matches('/').into(),
            credential: c.replication_credential_file.clone(),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout_read(Duration::from_secs(25))
                .redirects(0)
                .build(),
        })
    }
    pub(super) fn request(&self, path: &str) -> Result<ureq::Request> {
        if !matches!(path, "/replicate/stream" | "/replicate/challenge") {
            return Err(unknown("operation is outside replication transport"));
        }
        // Same private-file semantics as Quipu relay_auth::credential_header:
        // reread every operation; missing/rotated/invalid never uses old bytes.
        let bytes = private_file(&self.credential, 4096)?;
        let token = bytes
            .strip_suffix(b"\r\n")
            .or_else(|| bytes.strip_suffix(b"\n"))
            .unwrap_or(&bytes);
        if token.is_empty()
            || !token
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(b))
        {
            return Err(unknown(
                "replication credential must be one nonempty bearer line",
            ));
        }
        let token =
            std::str::from_utf8(token).map_err(|_| unknown("invalid replication credential"))?;
        Ok(
            super::super::provenance::apply_signed(
                self.agent.post(&format!("{}{path}", self.base)),
            )
            .set("Content-Type", "application/json")
            .set("Accept", "text/event-stream")
            .set("Authorization", &format!("Bearer {token}")),
        )
    }
}
