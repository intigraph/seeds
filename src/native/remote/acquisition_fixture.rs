//! Opt-in owned synthetic contract controls, never a production adapter.
use super::*;
use crate::model::{Comment, Seed};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
const PROJECT: &str = "https://example.org/project/fixture";
const EPHEMERAL: &str = "https://example.org/project/fixture#seeds-ephemeral";
const COLD: &str = "https://example.org/project/excluded";
const TIME: &str = "2026-01-01T00:00:00Z";
// These DTOs are not verifiers: the pinned producer validates the complete raw
// Payload before they are decoded. They expose only fields used by assertions.
#[derive(Deserialize)]
struct Payload {
    source_receipt: Receipt,
    source_receipt_sha256: String,
    graphs: BTreeMap<String, String>,
}
#[derive(Deserialize)]
struct Receipt {
    main: Main,
    composition: Composition,
}
#[derive(Deserialize)]
struct Main {
    applied_tx: i64,
}
#[derive(Deserialize)]
struct Composition {
    generation: String,
    providers: Vec<Provider>,
}
#[derive(Deserialize)]
struct Provider {
    exclusion: String,
}
struct Fixture {
    client: RemoteBackend,
    child: Child,
    dir: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self
            .client
            .post("/fixture/stop", "application/json", "{}", false);
        assert!(self.child.wait().unwrap().success());
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn obj(value: &Obj) -> Json {
    match value {
        Obj::Iri(s) => json!({"kind":"iri","value":s}),
        Obj::Str(s) => json!({"kind":"str","value":s}),
        Obj::Int(n) => json!({"kind":"int","value":n}),
        Obj::Typed { lexical, datatype } => {
            json!({"kind":"typed","value":lexical,"datatype":datatype})
        }
        Obj::Lang { .. } => panic!("unsupported fixture datum"),
    }
}
fn facts(f: &Fixture, graph: &str, subject: &str, facts: &[Fact], retract: bool) {
    let rows: Vec<_> = facts
        .iter()
        .map(|(p, o)| json!([subject, p, obj(o)]))
        .collect();
    f.call(
        "/fixture/facts",
        &json!({"graph":graph,"facts":rows,"retract":retract}),
    )
    .unwrap();
}
fn seed(id: &str, title: &str) -> Seed {
    Seed {
        id: id.into(),
        title: title.into(),
        status: "open".into(),
        priority: 2,
        issue_type: "task".into(),
        created_at: TIME.into(),
        updated_at: TIME.into(),
        revision: 1,
        ..Default::default()
    }
}
fn populate(fixture: &Fixture) {
    for (graph, s) in [
        (PROJECT, seed("fx-main", "before")),
        (EPHEMERAL, seed("fx-ephemeral", "private fixture")),
        (COLD, seed("fx-excluded", "excluded provider")),
    ] {
        facts(fixture, graph, &vocab::item_iri(&s.id), &s.facts(), false);
    }
    let comment = Comment {
        seed: "fx-main".into(),
        index: 1,
        author: "fixture".into(),
        text: "committed comment".into(),
        created_at: TIME.into(),
        extra: Default::default(),
    };
    facts(
        fixture,
        PROJECT,
        &vocab::comment_iri("fx-main", 1),
        &comment.facts(),
        false,
    );
}
impl Fixture {
    fn start() -> Self {
        let driver = std::env::var_os("SEEDS_ACQUISITION_FIXTURE_DRIVER")
            .expect("build pinned fixture producer first");
        let dir =
            std::env::temp_dir().join(format!("seeds-acquisition-fixture-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let mut child = Command::new(driver)
            .arg(&dir)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut base = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut base)
            .unwrap();
        assert!(base.starts_with("http://127.0.0.1:"));
        let mut client = RemoteBackend::connect(base.trim(), PROJECT, None, None, &[]).unwrap();
        client.agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(2))
            .build();
        let f = Self { client, child, dir };
        populate(&f);
        f.call("/fixture/freeze", &json!({})).unwrap();
        f
    }
    fn call(&self, route: &str, body: &Json) -> Result<String> {
        self.client
            .post(route, "application/json", &body.to_string(), false)
    }
    fn acquire(&self, request: &Json) -> Result<String> {
        self.call("/acquire/composed", request)
    }
}
fn refused(fixture: &Fixture, request: &Json) {
    let error = fixture.acquire(request).unwrap_err();
    // The existing transport represents a server's definite 422 refusal as
    // Failed; Unreachable is UNKNOWN and cannot satisfy a negative control.
    assert_eq!(error.kind, ErrorKind::Failed);
    assert!(error.message.contains("HTTP 422"), "{error}");
}
fn request(id: &str) -> Json {
    json!({"source_store_id":id,"graphs":[PROJECT,EPHEMERAL]})
}
fn verified(fixture: &Fixture, wire: &str, request: &Json) -> std::result::Result<Payload, String> {
    fixture
        .call("/fixture/verify", &json!({"request":request,"wire":wire}))
        .map_err(|e| e.to_string())?;
    let payload: Payload = serde_json::from_str(wire).map_err(|e| e.to_string())?;
    Ok(payload)
}
#[derive(Deserialize)]
struct RawEnvelope<'a> {
    #[serde(borrow)]
    source_receipt: &'a serde_json::value::RawValue,
    source_receipt_sha256: String,
}
fn persist(
    fixture: &Fixture,
    wire: &str,
    request: &Json,
    dir: &Path,
) -> std::result::Result<Payload, String> {
    let payload = verified(fixture, wire, request)?; // No writes before actual producer verification.
    let raw: RawEnvelope<'_> = serde_json::from_str(wire).map_err(|e| e.to_string())?;
    std::fs::create_dir(dir).map_err(|e| e.to_string())?;
    for (name, bytes) in [
        ("source-receipt.json", raw.source_receipt.get()),
        ("source-receipt.sha256", raw.source_receipt_sha256.as_str()),
    ] {
        std::fs::write(dir.join(name), bytes).map_err(|e| e.to_string())?;
    }
    // Artifact bytes and local metadata are separate from source authority.
    std::fs::write(dir.join("export.nt"), &payload.graphs[PROJECT]).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("artifact-receipt.json"),
        json!({"schema":"fixture-artifact/v1",
        "sha256":crate::pendant::sha256(payload.graphs[PROJECT].as_bytes()),
        "applied_main_tx":payload.source_receipt.main.applied_tx})
        .to_string(),
    )
    .map_err(|e| e.to_string())?;
    Ok(payload)
}
// Adversarial test input only: recompute the receipt hash to exercise semantic
// verification beyond the ordinary hash-tamper control. Never persisted as truth.
fn mutant(mut value: Json, pointer: &str, replacement: Json, rehash: bool) -> String {
    *value.pointer_mut(pointer).unwrap() = replacement;
    if rehash {
        value["source_receipt_sha256"] = json!(crate::pendant::sha256(
            &serde_json::to_vec(&value["source_receipt"]).unwrap()
        )
        .trim_start_matches("sha256:"));
    }
    value.to_string()
}

#[test]
#[ignore = "requires separately built pinned synthetic producer; see fixture README"]
#[cfg(target_os = "linux")]
fn bounded_acquisition_g2_g3_then_g4_g6() {
    let fixture = Fixture::start(); // Exactly one server and driver, synthetic owned store.
    let identity: Json =
        serde_json::from_str(&fixture.call("/fixture/id", &json!({})).unwrap()).unwrap();
    let id = identity["id"].as_str().unwrap();
    let current = request(id);
    let first_wire = fixture.acquire(&current).unwrap();
    let first = verified(&fixture, &first_wire, &current).unwrap();
    assert_eq!(first.graphs.len(), 2);
    assert!(first.graphs[PROJECT].contains("committed comment"));
    assert!(!first.graphs[EPHEMERAL].contains("committed comment"));
    assert!(!first.graphs[PROJECT].contains("excluded provider"));
    assert_eq!(first.source_receipt.composition.providers.len(), 1);
    assert_eq!(
        first.source_receipt.composition.providers[0].exclusion,
        "outside-qualified-main-scope"
    );
    for graphs in [
        json!([PROJECT, "https://example.org/absent#seeds-ephemeral"]),
        json!([PROJECT, "https://example.org/missing"]),
        json!([COLD]),
        json!([PROJECT, "https://example.org/project/fixture/comments"]),
    ] {
        let mut bad = current.clone();
        bad["graphs"] = graphs;
        refused(&fixture, &bad);
    }
    let mut bad = current.clone();
    bad["source_store_id"] = json!("foreign");
    refused(&fixture, &bad);
    let pin = first.source_receipt.main.applied_tx;
    let generation = first.source_receipt.composition.generation.clone();
    let pinned = json!({"source_store_id":id,"graphs":[PROJECT,EPHEMERAL],"tx":pin,"expected_generation":generation});
    // G3: observe a cut, retract/update, recover the exact old cut; never current fallback.
    facts(
        &fixture,
        PROJECT,
        &vocab::item_iri("fx-main"),
        &seed("fx-main", "before").facts(),
        true,
    );
    facts(
        &fixture,
        PROJECT,
        &vocab::item_iri("fx-main"),
        &seed("fx-main", "after").facts(),
        false,
    );
    let old_wire = fixture.acquire(&pinned).unwrap();
    let old = verified(&fixture, &old_wire, &pinned).unwrap();
    assert!(old.graphs[PROJECT].contains("\"before\""));
    assert!(!old.graphs[PROJECT].contains("\"after\""));
    let now = verified(&fixture, &fixture.acquire(&current).unwrap(), &current).unwrap();
    assert!(now.graphs[PROJECT].contains("\"after\""));
    assert!(!now.graphs[PROJECT].contains("\"before\""));
    assert_eq!(old.source_receipt.main.applied_tx, pin);
    for tx in [1, i64::MAX] {
        let mut bad = pinned.clone();
        bad["tx"] = json!(tx);
        refused(&fixture, &bad);
    }
    let mut bad = pinned.clone();
    bad.as_object_mut().unwrap().remove("expected_generation");
    refused(&fixture, &bad);
    fixture.call("/fixture/restart", &json!({})).unwrap();
    refused(&fixture, &pinned);
    let fresh_wire = fixture.acquire(&current).unwrap();
    let fresh = verified(&fixture, &fresh_wire, &current).unwrap();
    assert_ne!(fresh.source_receipt.composition.generation, generation);
    let mut fresh_pin = current.clone();
    fresh_pin["tx"] = json!(fresh.source_receipt.main.applied_tx);
    fresh_pin["expected_generation"] = json!(fresh.source_receipt.composition.generation);
    verified(&fixture, &fixture.acquire(&fresh_pin).unwrap(), &fresh_pin).unwrap();
    eprintln!("G2/G3 PASS: exact two-graph scope, comment ownership, provider exclusion, before/retract/after, floor/future/generation/restart negatives and new pin positive");

    // G4: keep the original raw receipt bytes, independent hash and artifact.
    let out = fixture.dir.join("accepted");
    let accepted = persist(&fixture, &fresh_wire, &current, &out).unwrap();
    let raw: RawEnvelope<'_> = serde_json::from_str(&fresh_wire).unwrap();
    assert_eq!(
        std::fs::read_to_string(out.join("source-receipt.json")).unwrap(),
        raw.source_receipt.get()
    );
    assert_eq!(
        std::fs::read_to_string(out.join("source-receipt.sha256")).unwrap(),
        accepted.source_receipt_sha256
    );
    let artifact: Json =
        serde_json::from_str(&std::fs::read_to_string(out.join("artifact-receipt.json")).unwrap())
            .unwrap();
    assert_eq!(
        artifact["sha256"],
        json!(crate::pendant::sha256(
            &std::fs::read(out.join("export.nt")).unwrap()
        ))
    );
    assert_eq!(
        artifact["applied_main_tx"],
        json!(accepted.source_receipt.main.applied_tx)
    );
    assert_ne!(artifact["schema"], json!("quipu-acquisition/composed-v1"));
    let parsed: Json = serde_json::from_str(&fresh_wire).unwrap();
    let cases = [
        ("/source_receipt/complete", json!(false)),
        ("/source_receipt/schema", json!("unknown/v1")),
        ("/source_receipt/main/applied_tx", json!(pin)),
        (
            "/source_receipt/composition/reader",
            json!("general-composed-query"),
        ),
        ("/source_receipt/composition/generation", json!("foreign")),
        (
            "/source_receipt/composition/providers/0/exclusion",
            json!("included"),
        ),
        (
            "/source_receipt/composition/ownership_sha256",
            json!("0".repeat(64)),
        ),
        ("/source_receipt/main/graphs", json!({})),
        (
            "/source_receipt/composition/providers/0/graphs",
            json!((0..513)
                .map(|n| format!("https://example.org/over-cap/{n}"))
                .collect::<Vec<_>>()),
        ),
        ("/graphs", json!({})),
    ];
    for (i, (pointer, replacement)) in cases.into_iter().enumerate() {
        let wire = mutant(parsed.clone(), pointer, replacement, true);
        let rejected = fixture.dir.join(format!("rejected-{i}"));
        let error = persist(&fixture, &wire, &current, &rejected)
            .err()
            .expect("mutant refused");
        assert!(
            error.contains("HTTP 422"),
            "mutant {pointer} must reach the actual producer verifier: {error}"
        );
        assert!(
            !rejected.exists(),
            "must refuse before artifact persistence"
        );
    }
    let mut missing = parsed.clone();
    missing["source_receipt"]
        .as_object_mut()
        .unwrap()
        .remove("composition");
    for wire in [
        missing.to_string(),
        fresh_wire[..fresh_wire.len() / 2].to_string(),
        mutant(
            parsed.clone(),
            "/source_receipt/complete",
            json!(false),
            false,
        ),
    ] {
        assert!(verified(&fixture, &wire, &current).is_err());
    }
    for route in ["/fixture/disconnect", "/fixture/timeout"] {
        let error = fixture.call(route, &json!({})).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unreachable);
        assert!(error.message.contains("does not fall back"));
    }
    verified(&fixture, &fixture.acquire(&current).unwrap(), &current).unwrap();
    // Unchanged ceilings: callers can only lower, never raise the byte ceiling.
    // Producer constants are pinned by the manifest; no cap changes here.
    let mut small = current.clone();
    small["max_bytes"] = json!(1);
    refused(&fixture, &small);
    eprintln!("G4 PASS: raw source receipt byte equality, separate artifact receipt, actual Payload::verify rejects hash/semantic/missing/truncated mutants before writes");

    if let Some(evidence) = std::env::var_os("SEEDS_ACQUISITION_FIXTURE_EVIDENCE") {
        let evidence = PathBuf::from(evidence);
        std::fs::create_dir(&evidence).unwrap();
        std::fs::write(evidence.join("wire.json"), &fresh_wire).unwrap();
        for entry in std::fs::read_dir(&out).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), evidence.join(entry.file_name())).unwrap();
        }
    }
    // G6 uses the actual Seeds parser/model, not string counts or a new reader.
    let main = crate::pendant::parse_ntriples(&accepted.graphs[PROJECT]).unwrap();
    let ephemeral = crate::pendant::parse_ntriples(&accepted.graphs[EPHEMERAL]).unwrap();
    let snapshot = Snapshot::from_graphs(&main, &ephemeral);
    assert_eq!(snapshot.seeds.len(), 2);
    assert_eq!(snapshot.comments.len(), 1);
    assert_eq!(snapshot.seeds["fx-main"].title, "after");
    assert!(snapshot.seeds["fx-ephemeral"].ephemeral);
    assert!(!snapshot.seeds.contains_key("fx-excluded"));
    assert_eq!(snapshot.comments[0].text, "committed comment");
    let without_ephemeral = Snapshot::from_graphs(&main, &BTreeMap::new());
    assert_ne!(
        snapshot.seeds.len(),
        without_ephemeral.seeds.len(),
        "scope mutant must break parity"
    );
    // The current Seeds parser cannot represent blank nodes. Preserve source
    // identity across graphs and explicitly refuse, never rename per graph.
    for graph in [PROJECT, EPHEMERAL] {
        facts(
            &fixture,
            graph,
            "_:shared",
            &[(
                "https://example.org/link".into(),
                Obj::Iri("_:shared".into()),
            )],
            false,
        );
    }
    let blank = verified(&fixture, &fixture.acquire(&current).unwrap(), &current).unwrap();
    assert!(blank.graphs[PROJECT].contains("_:shared"));
    assert!(blank.graphs[EPHEMERAL].contains("_:shared"));
    assert!(crate::pendant::parse_ntriples(&blank.graphs[PROJECT]).is_err());
    // The acquisition contract cannot be relabelled as an overlay, inferred,
    // dataset or SERVICE query. Unknown view selectors refuse at Request decode.
    for field in ["overlay", "inferred", "dataset", "service"] {
        let mut unsupported = current.clone();
        unsupported[field] = json!("https://example.org/other-view");
        refused(&fixture, &unsupported);
    }
    fixture.call("/fixture/dataset-alias", &json!({})).unwrap();
    refused(&fixture, &current);
    eprintln!("G6 PASS: main+committed-comment+ephemeral model parity; omitted-ephemeral mutant differs; excluded provider remains excluded; shared source blank node preserved and SDK unsupported refusal explicit");
}
