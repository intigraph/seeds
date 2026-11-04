//! Bounded same-origin requests. Never follow a redirect carrying a credential.
use super::Args;
use crate::error::{ErrorKind, Result, SdError};
use serde_json::Value;
use std::io::Read;

pub(super) struct Client {
    base: url::Url,
    agent: ureq::Agent,
    credential: super::super::credentials::Selected,
}
impl Client {
    pub fn new(endpoint: &str, args: &Args) -> Result<Self> {
        let base =
            url::Url::parse(endpoint).map_err(|_| SdError::usage("invalid adapter endpoint"))?;
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || !(base.scheme() == "https"
                || (base.scheme() == "http"
                    && matches!(base.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
        {
            return Err(SdError::refused("adapter requires HTTPS (HTTP only for loopback fixtures), no URL credentials/query/fragment"));
        }
        let credential = super::super::credentials::Selected::parse(
            args.transfer_principal.as_deref(),
            args.transfer_credential_fd.as_deref(),
        )?
        .ok_or_else(|| {
            SdError::usage("select a named transfer principal and inherited credential descriptor")
        })?
        .bind_destination(Some(endpoint))?;
        Ok(Self {
            base,
            credential,
            agent: ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(std::time::Duration::from_secs(20))
                .build(),
        })
    }
    pub fn url(&self, path: &str) -> Result<String> {
        let mut out = self.base.clone();
        out.set_path(&format!(
            "{}/{}",
            self.base.path().trim_end_matches('/'),
            path.trim_start_matches('/')
        ));
        Ok(out.to_string())
    }
    pub fn request(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value> {
        let url = self.url(path)?;
        let secret = self.credential.read()?;
        let mut req = self
            .agent
            .request(method, &url)
            .set("Accept", "application/json")
            .set("User-Agent", "seeds-task-transfer");
        req = req
            .set("Authorization", &format!("Bearer {secret}"))
            .set("X-GitHub-Api-Version", "2026-03-10");
        for (key, value) in query {
            req = req.query(key, value);
        }
        let response = if let Some(body) = body {
            req.set("Content-Type", "application/json")
                .send_string(&body.to_string())
        } else {
            req.call()
        };
        let response = response.map_err(|error| match error {
            ureq::Error::Status(code, _) => SdError::new(
                ErrorKind::Refused,
                format!("task-manager HTTP {code}; no fallback or automatic retry"),
            ),
            _ => SdError::new(
                if method == "GET" {
                    ErrorKind::Unreachable
                } else {
                    ErrorKind::Indeterminate
                },
                "task-manager transport failed; read back before retrying a write",
            ),
        })?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                SdError::new(
                    ErrorKind::Indeterminate,
                    "task-manager response incomplete; read back before retry",
                )
            })?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(SdError::refused("task-manager response exceeds 2 MiB"));
        }
        if status == 204 && bytes.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            SdError::new(
                if method == "GET" {
                    ErrorKind::Refused
                } else {
                    ErrorKind::Indeterminate
                },
                "task-manager returned invalid JSON; read back before retrying a write",
            )
        })
    }
}
