//! Bounded requests on the explicitly admitted owner view.
use super::*;
use base64::Engine as _;
use std::io::Read;
use std::time::Duration;

fn body(response: ureq::Response) -> Result<String> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("response body unavailable"))?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(invalid("response byte bound"));
    }
    String::from_utf8(bytes).map_err(|_| invalid("response is not UTF-8"))
}
fn http_error(error: ureq::Error) -> SdError {
    match error {
        ureq::Error::Status(code, _) => {
            invalid("authenticated request refused").with_details(json!({"http_status":code}))
        }
        ureq::Error::Transport(_) => SdError::new(
            ErrorKind::Unreachable,
            "graph-set request unavailable; no fallback",
        ),
    }
}
impl Mode {
    fn request(&self, remote: &RemoteBackend, path: &str, read: bool) -> ureq::Request {
        crate::native::result::request_phase(!read);
        let request = remote.agent.post(&format!("{}{path}", self.profile.base));
        let request = if read {
            quipu_read_request(request, "graph-set")
        } else {
            quipu_request(request)
        };
        request
            .timeout(Duration::from_secs(10))
            .set("Content-Type", "application/json")
            .set("Accept", "application/sparql-results+json")
            .set(
                "Authorization",
                &format!("Bearer {}", remote.token.as_ref().unwrap()),
            )
    }
    pub(super) fn preflight(&self, remote: &RemoteBackend) -> Result<()> {
        let response = quipu_read_request(
            remote.agent.get(&format!("{}/version", self.profile.base)),
            "version",
        )
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(http_error)?;
        let version: Value =
            serde_json::from_str(&body(response)?).map_err(|_| invalid("version response"))?;
        let c = &version["routed_update_completion"];
        if c["operation"] != "/update"
            || c["enabled"] != true
            || c["request_contract"] != REQUEST_CONTRACT
            || c["status_contract"] != STATUS_CONTRACT
            || c["status"] != "/update/status"
            || c["seal"] != "/update/seal"
        {
            return Err(invalid("live routed update completion is not admitted"));
        }
        let answer = self.query(remote, &json!({"query":"ASK {}"}).to_string())?;
        let answer: Value =
            serde_json::from_str(&answer).map_err(|_| invalid("read control response"))?;
        if answer["boolean"] != true {
            return Err(invalid("authorized read positive control"));
        }
        let denied = self.request(remote, "/graph-sets/query", true).send_string(
            &json!({
                "query":"ASK {}", "graphs":[self.profile.denied_control_graph]
            })
            .to_string(),
        );
        match denied {
            Err(ureq::Error::Status(403, response)) => {
                let answer: Value = serde_json::from_str(&body(response)?)
                    .map_err(|_| invalid("denied control response"))?;
                if answer != json!({"error":"composed read denied"}) {
                    return Err(invalid("denied control must expose only one denial"));
                }
            }
            _ => return Err(invalid("ungranted graph control did not refuse 403")),
        }
        Ok(())
    }
    pub(in crate::native::remote) fn query(
        &self,
        remote: &RemoteBackend,
        original: &str,
    ) -> Result<String> {
        self.query_with_cut(remote, original, None)
            .map(|(body, _)| body)
    }
    pub(super) fn query_with_cut(
        &self,
        remote: &RemoteBackend,
        original: &str,
        minimum: Option<u64>,
    ) -> Result<(String, String)> {
        let input: Value = serde_json::from_str(original).map_err(|_| invalid("query input"))?;
        let object = input
            .as_object()
            .ok_or_else(|| invalid("query must be an object"))?;
        if object.len() != 1 {
            return Err(invalid(
                "historical/default dataset options unsupported by owner read",
            ));
        }
        let query = input["query"]
            .as_str()
            .filter(|q| q.len() <= 65536)
            .ok_or_else(|| invalid("query byte bound"))?;
        let response = self
            .request(remote, "/graph-sets/query", true)
            .send_string(&json!({"query":query,"graphs":self.profile.members()}).to_string())
            .map_err(http_error)?;
        let cut = response
            .header("quipu-composition-cut")
            .ok_or_else(|| invalid("composed answer has no cut"))?;
        self.check_cut(cut)?;
        let cut = cut.to_string();
        if let Some(minimum) = minimum {
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(cut.strip_prefix("qcut1.").unwrap())
                .map_err(|_| invalid("cut encoding"))?;
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid("cut JSON"))?;
            if value["members"]
                .as_object()
                .unwrap()
                .values()
                .any(|m| m["local_tx"].as_u64().unwrap() < minimum)
            {
                return Err(invalid("proof cut is behind the commit witness"));
            }
        }
        let text = body(response)?;
        let value: Value =
            serde_json::from_str(&text).map_err(|_| invalid("composed answer JSON"))?;
        if value.get("error").is_some()
            || value.get("truncated").is_some_and(|v| v != false)
            || (value.get("boolean").and_then(Value::as_bool).is_none()
                && !value["results"]["bindings"].is_array())
        {
            return Err(invalid("composed answer incomplete or malformed"));
        }
        Ok((text, cut))
    }
    pub(super) fn check_cut(&self, token: &str) -> Result<()> {
        self.check_cut_for(token, &self.profile.members(), None)
    }
    pub(super) fn check_cut_for(
        &self,
        token: &str,
        selected: &[String],
        minimum: Option<u64>,
    ) -> Result<()> {
        if token.len() > 16384 {
            return Err(invalid("cut byte bound"));
        }
        let encoded = token
            .strip_prefix("qcut1.")
            .ok_or_else(|| invalid("cut version"))?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| invalid("cut encoding"))?;
        let cut: Value = serde_json::from_slice(&bytes).map_err(|_| invalid("cut JSON"))?;
        if serde_json::to_vec(&cut).map_err(|_| invalid("cut canonicalization"))? != bytes
            || cut["version"] != 1
            || cut["catalog_revision"].as_u64().is_none()
            || cut.as_object().is_none_or(|o| o.len() != 3)
        {
            return Err(invalid("noncanonical or malformed cut"));
        }
        let members = cut["members"]
            .as_object()
            .ok_or_else(|| invalid("cut members"))?;
        if selected.is_empty() || members.len() != selected.len() {
            return Err(invalid("cut selection differs"));
        }
        for graph in selected {
            let m = members
                .get(graph)
                .ok_or_else(|| invalid("cut selection differs"))?;
            if m["store_id"] != self.profile.store_id
                || m["log_epoch"] != self.profile.log_epoch
                || m["local_tx"].as_u64().is_none()
                || minimum.is_some_and(|tx| m["local_tx"].as_u64().is_none_or(|n| n < tx))
                || m["generation"].as_u64().is_none()
                || m["policy"].as_str().is_none_or(|p| !identifier(p))
                || m.as_object().is_none_or(|o| o.len() != 5)
            {
                return Err(invalid("foreign store/epoch or malformed cut member"));
            }
        }
        Ok(())
    }
    pub(super) fn status(
        &self,
        remote: &RemoteBackend,
        attempt: &Attempt,
        seal: bool,
    ) -> Result<Value> {
        let response = self
            .request(
                remote,
                if seal {
                    "/update/seal"
                } else {
                    "/update/status"
                },
                !seal,
            )
            .send_string(&json!({"write_id":attempt.write_id,"update":attempt.update}).to_string())
            .map_err(http_error)?;
        let status: Value =
            serde_json::from_str(&body(response)?).map_err(|_| invalid("status JSON"))?;
        if status["contract"] != STATUS_CONTRACT || status["write_id"] != attempt.write_id {
            return Err(invalid("status contract or binding mismatch"));
        }
        if status["state"] != "unknown"
            && (status["store_id"] != self.profile.store_id
                || status["log_epoch"] != self.profile.log_epoch)
        {
            return Err(invalid("status lineage mismatch"));
        }
        Ok(status)
    }
    pub(super) fn send_attempt(&self, remote: &RemoteBackend, attempt: &Attempt) -> Result<()> {
        // The exact form and ID are already durable. Never re-run this on recovery.
        let result = self
            .request(remote, "/update", false)
            .set("Content-Type", "application/x-www-form-urlencoded")
            .set("X-Quipu-Completion-Contract", REQUEST_CONTRACT)
            .set("X-Quipu-Write-Id", &attempt.write_id)
            .send_string(&attempt.form);
        match result {
            Ok(response) => {
                body(response)?;
                Ok(())
            }
            Err(error) => Err(http_error(error)),
        }
    }
}
