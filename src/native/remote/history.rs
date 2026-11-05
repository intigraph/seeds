//! Explicit retained MAIN fact history, never a composed version-history proof.
//! The caller supplies trusted source lineage. Responses cannot enroll a source.
use super::{quipu_read_request, RemoteBackend};
use crate::error::{ErrorKind, Result, SdError};
use serde::Serialize;
use serde_json::Value;
use std::io::Read;
use std::time::{Duration, Instant};

const CONTRACT: &str = "quipu-entity-history/main-v1";
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// An exact subject/graph/range in the caller's trusted MAIN source.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryRequest {
    pub iri: String,
    pub graph: String,
    pub source_store_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_generation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_tx: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until_tx: Option<u64>,
    pub max_entries: usize,
    pub max_bytes: usize,
}

/// Validated retained facts plus the exact source wire, not re-encoded JSON.
/// Completeness covers this physical MAIN range only, not continuous seed history.
pub struct RetainedHistory {
    request: Vec<u8>,
    wire: Vec<u8>,
    response: Value,
}
impl RetainedHistory {
    pub fn request_bytes(&self) -> &[u8] {
        &self.request
    }
    pub fn source_bytes(&self) -> &[u8] {
        &self.wire
    }
    pub fn response(&self) -> &Value {
        &self.response
    }
}

fn invalid(why: &str) -> SdError {
    SdError::refused(format!("retained history UNKNOWN: {why}; no fallback"))
}
fn unavailable(why: &str) -> SdError {
    SdError::new(
        ErrorKind::Unreachable,
        format!("retained history UNKNOWN: {why}; no fallback"),
    )
}
fn http_error(e: ureq::Error) -> SdError {
    match e {
        ureq::Error::Transport(_) => unavailable("transport"),
        ureq::Error::Status(c, _) if c == 408 || c == 429 || c >= 500 => {
            unavailable("server/deadline")
        }
        _ => invalid("server refused contract/request"),
    }
}
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| unavailable("total deadline"))
}
fn bounded(mut reader: impl Read, cap: usize, deadline: Instant) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0; 8192];
    loop {
        remaining(deadline)?;
        let n = reader
            .read(&mut buf)
            .map_err(|_| unavailable("body read"))?;
        remaining(deadline)?;
        if n == 0 {
            return Ok(out);
        }
        if out.len().saturating_add(n) > cap {
            return Err(invalid("wire byte cap"));
        }
        out.extend_from_slice(&buf[..n]);
    }
}
fn digest(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.strip_prefix("sha256:").is_some_and(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    })
}
fn natural(v: &Value) -> Option<u64> {
    v.as_u64().filter(|n| *n <= i64::MAX as u64)
}
fn iri(v: &str) -> bool {
    !v.is_empty() && v.len() <= 8192 && oxiri::Iri::parse(v).is_ok()
}
impl HistoryRequest {
    fn validate(&self, project: &str) -> Result<()> {
        let ephemeral = format!("{project}#seeds-ephemeral");
        if !iri(&self.iri)
            || !iri(&self.graph)
            || self.graph == "urn:quipu:graph:root"
            || (self.graph != project && self.graph != ephemeral)
            || self.source_store_id.is_empty()
            || self.source_store_id.len() > 256
            || !(1..=10000).contains(&self.max_entries)
            || !(1..=MAX_BYTES).contains(&self.max_bytes)
            || self
                .expected_generation
                .as_ref()
                .is_some_and(|s| !digest(&Value::String(s.clone())))
            || (self.since_tx.is_some() || self.until_tx.is_some())
                && self.expected_generation.is_none()
            || self.since_tx.is_some_and(|n| n > i64::MAX as u64)
            || self.until_tx.is_some_and(|n| n == 0 || n > i64::MAX as u64)
            || self.since_tx.zip(self.until_tx).is_some_and(|(a, b)| a > b)
        {
            return Err(invalid(
                "request scope, identity, generation, bounds or budget",
            ));
        }
        Ok(())
    }
}
fn typed(v: &Value) -> bool {
    let text = || v["value"].as_str();
    match v["type"].as_str() {
        Some("string") => text().is_some(),
        Some("iri") => text().is_some_and(iri),
        Some("integer") => {
            text().is_some_and(|s| s.parse::<i64>().is_ok_and(|n| n.to_string() == s))
        }
        Some("boolean") => v["value"].is_boolean(),
        Some("float64") => v["bits"].as_str().is_some_and(|s| {
            s.len() == 16
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }),
        Some("bytes") => {
            v["encoding"] == "hex"
                && text().is_some_and(|s| {
                    s.len() % 2 == 0
                        && s.bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
        }
        Some("lang") => text().is_some() && v["lang"].as_str().is_some_and(|s| !s.is_empty()),
        Some("typed") => text().is_some() && v["datatype"].as_str().is_some_and(iri),
        _ => false,
    }
}
fn validate_response(request: &HistoryRequest, wire: Vec<u8>) -> Result<RetainedHistory> {
    if wire.len() > request.max_bytes {
        return Err(invalid("wire byte cap"));
    }
    let response: Value = serde_json::from_slice(&wire).map_err(|_| invalid("response JSON"))?;
    let r = &response["source_receipt"];
    let rows = response["history"]
        .as_array()
        .ok_or_else(|| invalid("history array absent"))?;
    let floor = natural(&r["retained_log_min_tx"])
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("floor"))?;
    let head = natural(&r["observed_head_tx"])
        .filter(|n| *n >= floor)
        .ok_or_else(|| invalid("head"))?;
    let since = natural(&r["since_tx_exclusive"]).ok_or_else(|| invalid("since bound"))?;
    let until = natural(&r["until_tx_inclusive"]).ok_or_else(|| invalid("until bound"))?;
    if response["schema"] != CONTRACT
        || response["iri"] != request.iri
        || response["graph"] != request.graph
        || response.get("error").is_some()
        || response
            .get("truncated")
            .is_some_and(|v| v != &Value::Bool(false))
        || response["count"].as_u64() != Some(rows.len() as u64)
        || rows.len() > request.max_entries
        || r["provider"] != "main"
        || r["source_store_id"] != request.source_store_id
        || !digest(&r["generation"])
        || !digest(&r["incarnation"])
        || r["complete"] != true
        || r["coverage"] != "retained-main-physical-facts"
        || r["continuous_history"] != false
        || r["historical_policy_proof"] != false
        || !r["excluded_attachment_aliases"]
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string))
        || since != request.since_tx.unwrap_or(floor - 1)
        || until != request.until_tx.unwrap_or(head)
        || since < floor - 1
        || since > until
        || until > head
        || until < floor
        || request
            .expected_generation
            .as_ref()
            .is_some_and(|g| r["generation"] != *g)
    {
        return Err(invalid(
            "source receipt, scope, completeness or range mismatch",
        ));
    }
    let mut previous = since;
    for row in rows {
        let q = &row["qualified_tx"];
        let tx = natural(&q["local_tx"]).ok_or_else(|| invalid("row transaction"))?;
        if q["provider"] != "main"
            || q["source_store_id"] != r["source_store_id"]
            || q["generation"] != r["generation"]
            || tx <= since
            || tx > until
            || tx < previous
            || !row["predicate"].as_str().is_some_and(iri)
            || !typed(&row["value"])
            || !matches!(row["op"].as_str(), Some("assert" | "retract"))
            || !row
                .get("valid_to")
                .is_some_and(|v| v.is_null() || v.is_string())
            || !row["valid_from"].is_string()
            || !row["transaction"]["timestamp"].is_string()
            || !row
                .get("authenticated")
                .is_some_and(|v| v.is_null() || v.is_object())
            || !row
                .get("declared")
                .is_some_and(|v| v.is_null() || v.is_object())
        {
            return Err(invalid("row identity, ordering, type or evidence mismatch"));
        }
        previous = tx;
    }
    let encoded = serde_json::to_vec(request).map_err(|_| invalid("request JSON"))?;
    Ok(RetainedHistory {
        request: encoded,
        wire,
        response,
    })
}
impl RemoteBackend {
    /// Explicit SDK read; never called implicitly by `sd history` or export.
    /// No retry, ROOT fallback, inferred source enrollment or version reconstruction.
    pub fn retained_history(
        &self,
        request: &HistoryRequest,
        timeout: Duration,
    ) -> Result<RetainedHistory> {
        request.validate(&self.graph)?;
        if timeout.is_zero() || timeout > Duration::from_secs(30) {
            return Err(invalid("total timeout must be (0,30s]"));
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| invalid("deadline"))?;
        let version = quipu_read_request(
            self.agent.get(&format!("{}/version", self.base)),
            "history-version",
        )
        .timeout(remaining(deadline)?)
        .call()
        .map_err(http_error)?;
        let advertised: Value =
            serde_json::from_slice(&bounded(version.into_reader(), 65536, deadline)?)
                .map_err(|_| invalid("version JSON"))?;
        if !advertised["read_contracts"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == CONTRACT))
        {
            return Err(invalid("contract not advertised"));
        }
        let body = serde_json::to_vec(request).map_err(|_| invalid("request JSON"))?;
        let response = quipu_read_request(
            self.agent.post(&format!("{}/entity_history", self.base)),
            "history",
        )
        .timeout(remaining(deadline)?)
        .set("Content-Type", "application/json")
        .send_bytes(&body)
        .map_err(http_error)?;
        let history = validate_response(
            request,
            bounded(response.into_reader(), request.max_bytes, deadline)?,
        )?;
        remaining(deadline)?;
        Ok(history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/retained-main-history.json"
        ))
        .unwrap()
    }
    fn request() -> HistoryRequest {
        HistoryRequest {
            iri: "https://example.org/item/one".into(),
            graph: "https://example.org/project/board".into(),
            source_store_id: "urn:uuid:e13a398a-54d1-454d-82ec-a7fa7cb14ef8".into(),
            expected_generation: None,
            since_tx: None,
            until_tx: None,
            max_entries: 1000,
            max_bytes: 1024 * 1024,
        }
    }
    fn check(value: &Value) -> Result<RetainedHistory> {
        validate_response(&request(), serde_json::to_vec(value).unwrap())
    }
    #[test]
    fn recorded_assert_retract_preserves_source_bytes_and_separates_declarations() {
        let wire = include_bytes!("../../../tests/fixtures/retained-main-history.json").to_vec();
        let got = validate_response(&request(), wire.clone()).unwrap();
        assert_eq!(got.source_bytes(), wire);
        assert_eq!(
            serde_json::from_slice::<Value>(got.request_bytes()).unwrap()["graph"],
            request().graph
        );
        assert_eq!(got.response()["history"][0]["authenticated"], Value::Null);
        assert_eq!(
            got.response()["history"][0]["declared"]["host"],
            "unit-host"
        );
        assert_eq!(
            got.response()["source_receipt"]["continuous_history"],
            false
        );
        assert_eq!(got.response()["history"][1]["op"], "retract");
    }
    #[test]
    fn scope_completeness_lineage_generation_and_ranges_are_not_advisory() {
        for (pointer, replacement) in [
            ("/schema", json!("unknown")),
            ("/iri", json!("https://example.org/wrong")),
            ("/graph", json!("urn:quipu:graph:root")),
            ("/count", json!(0)),
            ("/source_receipt/provider", json!("attachment")),
            ("/source_receipt/source_store_id", json!("foreign")),
            ("/source_receipt/generation", json!("unqualified")),
            ("/source_receipt/incarnation", json!(null)),
            ("/source_receipt/complete", json!(false)),
            ("/source_receipt/coverage", json!("all")),
            ("/source_receipt/continuous_history", json!(true)),
            ("/source_receipt/historical_policy_proof", json!(true)),
            ("/source_receipt/retained_log_min_tx", json!(0)),
            ("/source_receipt/observed_head_tx", json!(0)),
            ("/source_receipt/since_tx_exclusive", json!(1)),
            ("/source_receipt/until_tx_inclusive", json!(3)),
            ("/source_receipt/excluded_attachment_aliases", json!([9])),
            ("/history/0/qualified_tx/source_store_id", json!("foreign")),
            (
                "/history/0/qualified_tx/generation",
                json!(format!("sha256:{}", "0".repeat(64))),
            ),
            ("/history/0/qualified_tx/provider", json!("other")),
            ("/history/0/qualified_tx/local_tx", json!(0)),
            ("/history/1/qualified_tx/local_tx", json!(3)),
            ("/history/0/op", json!("update")),
            ("/history/0/predicate", json!("a:alias with space")),
            ("/history/0/value/type", json!("unknown")),
            ("/history/0/transaction/timestamp", json!(null)),
            ("/history/0/declared", json!([])),
            ("/history/0/authenticated", json!("unit-host")),
        ] {
            let mut v = fixture();
            *v.pointer_mut(pointer).unwrap() = replacement;
            assert!(check(&v).is_err(), "accepted mutation {pointer}");
        }
        for field in ["error", "truncated"] {
            let mut v = fixture();
            v[field] = json!(true);
            assert!(check(&v).is_err());
        }
        let mut v = fixture();
        v["history"].as_array_mut().unwrap().reverse();
        assert!(check(&v).is_err());
    }
    #[test]
    fn explicit_range_requires_observed_generation_and_receipt_matches_it() {
        let mut r = request();
        r.since_tx = Some(0);
        r.until_tx = Some(2);
        assert!(r.validate(&r.graph).is_err());
        r.expected_generation = Some(
            fixture()["source_receipt"]["generation"]
                .as_str()
                .unwrap()
                .into(),
        );
        assert!(r.validate(&r.graph).is_ok());
        let wire = serde_json::to_vec(&fixture()).unwrap();
        assert!(validate_response(&r, wire.clone()).is_ok());
        r.expected_generation = Some(format!("sha256:{}", "1".repeat(64)));
        assert!(validate_response(&r, wire).is_err());
    }
    #[test]
    fn request_admission_refuses_foreign_graph_bad_identity_and_widened_budgets() {
        let base = request();
        assert!(base.validate(&base.graph).is_ok());
        let mut r = base.clone();
        r.graph.push_str("#seeds-ephemeral");
        assert!(r.validate(&base.graph).is_ok());
        let mut r = base.clone();
        r.graph = "https://example.org/foreign".into();
        assert!(r.validate(&base.graph).is_err());
        let mut r = base.clone();
        r.source_store_id.clear();
        assert!(r.validate(&base.graph).is_err());
        let mut r = base.clone();
        r.max_entries = 10001;
        assert!(r.validate(&base.graph).is_err());
        let mut r = base.clone();
        r.max_bytes = MAX_BYTES + 1;
        assert!(r.validate(&base.graph).is_err());
        let mut r = base.clone();
        r.until_tx = Some(u64::MAX);
        assert!(r.validate(&base.graph).is_err());
        let mut r = base.clone();
        r.iri = "not an iri".into();
        assert!(r.validate(&base.graph).is_err());
    }
    #[test]
    fn byte_row_caps_and_deadline_refuse_instead_of_truncating() {
        let mut r = request();
        r.max_entries = 1;
        assert!(validate_response(&r, serde_json::to_vec(&fixture()).unwrap()).is_err());
        let mut r = request();
        r.max_bytes = 20;
        assert!(validate_response(&r, serde_json::to_vec(&fixture()).unwrap()).is_err());
        let deadline = Instant::now() + Duration::from_secs(1);
        assert!(bounded(&b"12345"[..], 4, deadline).is_err());
        assert!(bounded(&b""[..], 4, Instant::now()).is_err());
    }
    #[test]
    fn all_wire_value_types_are_recognized_without_losing_representation() {
        for value in [
            json!({"type":"integer","value":"-9223372036854775808"}),
            json!({"type":"float64","bits":"8000000000000000"}),
            json!({"type":"bytes","encoding":"hex","value":"00ff"}),
            json!({"type":"iri","value":"https://example.org/object"}),
            json!({"type":"boolean","value":false}),
            json!({"type":"lang","value":"text","lang":"en"}),
            json!({"type":"typed","value":"01.00","datatype":"http://www.w3.org/2001/XMLSchema#decimal"}),
        ] {
            let mut v = fixture();
            v["history"][0]["value"] = value.clone();
            assert_eq!(check(&v).unwrap().response()["history"][0]["value"], value);
        }
        assert!(!typed(&json!({"type":"integer","value":1})));
        assert!(!typed(&json!({"type":"float64","bits":"0"})));
        assert!(!typed(
            &json!({"type":"bytes","encoding":"hex","value":"f"})
        ));
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    fn exercise(advertised: bool, status: u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let mut seen = Vec::new();
            let expected = if advertised { 3 } else { 2 };
            let deadline = Instant::now() + Duration::from_secs(5);
            while seen.len() < expected {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing expected client request");
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("fixture accept: {e}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_string();
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((key, value)) = line.split_once(':') {
                        if key.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap();
                        }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let (code,reply)=match path.as_str() {
                    "/health"=>(200,b"{\"status\":\"ok\"}".to_vec()),
                    "/version"=>(200,serde_json::to_vec(&serde_json::json!({"read_contracts":if advertised {vec![CONTRACT]} else {vec![]}})).unwrap()),
                    "/entity_history"=> {
                        let request:Value=serde_json::from_slice(&body).unwrap();
                        assert_eq!(request["source_store_id"],"urn:uuid:e13a398a-54d1-454d-82ec-a7fa7cb14ef8");
                        assert_eq!(request["graph"],"https://example.org/project/board");
                        (status,if status==200 { include_bytes!("../../../tests/fixtures/retained-main-history.json").to_vec() } else { b"{\"error\":\"generation refused\"}".to_vec() })
                    },
                    _=>panic!("unexpected endpoint/fallback: {path}"),
                };
                seen.push(path);
                write!(stream,"HTTP/1.1 {code} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",reply.len()).unwrap();
                stream.write_all(&reply).unwrap();
            }
            seen
        });
        let backend =
            RemoteBackend::connect(&url, "https://example.org/project/board", None, None, &[])
                .unwrap();
        let request = HistoryRequest {
            iri: "https://example.org/item/one".into(),
            graph: "https://example.org/project/board".into(),
            source_store_id: "urn:uuid:e13a398a-54d1-454d-82ec-a7fa7cb14ef8".into(),
            expected_generation: None,
            since_tx: None,
            until_tx: None,
            max_entries: 1000,
            max_bytes: 1024 * 1024,
        };
        let result = backend.retained_history(&request, Duration::from_secs(3));
        assert_eq!(result.is_ok(), advertised && status == 200);
        let seen = worker.join().unwrap();
        assert_eq!(
            seen,
            if advertised {
                vec!["/health", "/version", "/entity_history"]
            } else {
                vec!["/health", "/version"]
            }
        );
    }
    #[test]
    fn actual_sdk_gets_contract_then_exact_bounded_history() {
        exercise(true, 200);
    }
    #[test]
    fn unsupported_contract_never_requests_history() {
        exercise(false, 200);
    }
    #[test]
    fn server_refusal_has_no_root_query_retry_or_current_fallback() {
        exercise(true, 400);
    }
}
