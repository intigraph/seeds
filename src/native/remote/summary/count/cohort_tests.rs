//! Execute the emitted query against a private engine and an independent model.
use super::*;
use crate::backend::Backend;
use crate::quipu_backend::QuipuBackend;
use quipu::store::{Datum, Store};
use quipu::types::{Op, Value};

const GRAPH: &str = "https://example.org/count-board";
const NOW: &str = "2026-10-01T00:00:00Z";

fn seed(store: &mut Store, graph: &str, subject: &str, id: &str, fields: Vec<(String, Value)>) {
    let graph = store.graph_create(graph).unwrap();
    let entity = store.intern(subject).unwrap();
    let class = Value::Ref(store.intern(&term::work_item()).unwrap());
    let mut fields = fields;
    fields.extend([
        (vocab::RDF_TYPE.into(), class),
        (term::identifier(), Value::Str(id.into())),
    ]);
    let datums = fields
        .into_iter()
        .map(|(p, value)| Datum {
            entity,
            attribute: store.intern(&p).unwrap(),
            value,
            valid_from: NOW.into(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect::<Vec<_>>();
    store
        .transact_to_graph(&datums, NOW, None, None, graph)
        .unwrap();
}

fn fields(
    status: Option<&str>,
    priority: Option<i64>,
    actor: Option<&str>,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if let Some(s) = status {
        out.push((term::status(), Value::Str(s.into())));
    }
    if let Some(p) = priority {
        out.push((term::priority(), Value::Int(p)));
    }
    if let Some(a) = actor {
        out.push((term::assigned_to(), Value::Str(a.into())));
    }
    out
}

fn fixture() -> QuipuBackend {
    // Deliberately irregular historical facts exercise the reader contract.
    // Build an isolated raw store, then use the read-only model backend; the
    // normal writer correctly refuses several of these inputs.
    let mut private = Store::open_in_memory().unwrap();
    let store = &mut private;
    for (subject, id, status, priority, actor) in [
        ("a", "a", Some("open"), Some(1), Some("reviewer")),
        ("duplicate-a", "a", Some("open"), Some(1), Some("reviewer")),
        ("other-actor", "b", Some("open"), Some(1), Some("other")),
        (
            "other-priority",
            "c",
            Some("open"),
            Some(2),
            Some("reviewer"),
        ),
        ("shadow", "shadow", Some("open"), Some(1), Some("reviewer")),
        ("default", "default", None, None, None),
        (
            "tombstone",
            "tombstone",
            Some("tombstone"),
            Some(1),
            Some("reviewer"),
        ),
        (
            "closed",
            "closed",
            Some("closed"),
            Some(1),
            Some("reviewer"),
        ),
        (
            "deferred",
            "deferred",
            Some("deferred"),
            Some(1),
            Some("reviewer"),
        ),
    ] {
        let mut values = fields(status, priority, actor);
        if id == "a" {
            values.extend(
                [("alpha", term::label()), ("beta", term::label())]
                    .into_iter()
                    .map(|(label, predicate)| (predicate, Value::Str(label.into()))),
            );
        }
        seed(store, GRAPH, &format!("urn:count:{subject}"), id, values);
    }
    let ephemeral = vocab::ephemeral_graph(GRAPH);
    seed(
        store,
        &ephemeral,
        "urn:count:replacement",
        "shadow",
        fields(Some("closed"), Some(1), Some("reviewer")),
    );
    seed(
        store,
        &ephemeral,
        "urn:count:ephemeral",
        "ephemeral",
        fields(Some("open"), Some(1), Some("reviewer")),
    );
    seed(
        store,
        "https://example.org/inbox",
        "urn:count:inbox",
        "inbox",
        fields(Some("open"), Some(1), Some("reviewer")),
    );
    QuipuBackend::read_only(private, GRAPH)
}

fn rows(store: &Store, query: &str) -> Vec<BTreeMap<String, Obj>> {
    quipu::sparql::query(store, query)
        .unwrap()
        .rows()
        .iter()
        .map(|row| {
            row.iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::Str(s) => Obj::Str(s.clone()),
                        Value::Int(n) => Obj::Int(*n),
                        Value::Ref(r) => Obj::Iri(store.resolve(*r).unwrap()),
                        _ => panic!("unexpected aggregate value: {v:?}"),
                    };
                    (k.clone(), v)
                })
                .collect()
        })
        .collect()
}

#[test]
fn count_snapshot_effective_cohort_membership_and_model_parity() {
    let (base, worker) = crate::native::remote::gateway_tests::server(vec![(200, "ok")]);
    let remote = RemoteBackend::connect(&base, GRAPH, None, None, &[]).unwrap();
    worker.join().unwrap();
    let local = fixture();
    let filter = Filter {
        assignee: Some("reviewer".into()),
        priority: Some("P1".into()),
        ..Default::default()
    };
    let fields = remote.filter_fields(&filter).unwrap()
        + " FILTER(?status != \"closed\" && ?status != \"tombstone\")";
    let membership = rows(
        local.store(),
        &format!(
            "SELECT DISTINCT ?id WHERE {{ {} }}",
            remote.effective_normalized(&fields)
        ),
    )
    .into_iter()
    .map(|row| match row.get("id").unwrap() {
        Obj::Str(id) => id.clone(),
        _ => panic!(),
    })
    .collect::<BTreeSet<_>>();
    assert_eq!(
        membership,
        ["a", "deferred", "ephemeral"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
    for filter in [Filter::default(), filter] {
        for by in [
            None,
            Some("status"),
            Some("priority"),
            Some("assignee"),
            Some("type"),
            Some("label"),
        ] {
            let req = CountReq {
                filter: filter.clone(),
                by: by.map(str::to_string),
                include_closed: false,
            };
            let expected = engine::count(&local, &req, None).unwrap();
            let actual = decode_count(
                rows(local.store(), &remote.count_query(&req).unwrap()),
                false,
                by,
            )
            .unwrap();
            assert_eq!(actual.total, expected.total, "{req:?}");
            assert_eq!(actual.groups, expected.groups, "{req:?}");
        }
    }
    // The fixture must also retain the replacement rather than the main row.
    assert_eq!(
        local.snapshot(None).unwrap().seeds["shadow"].status,
        "closed"
    );
}

#[test]
fn count_snapshot_multivalue_filters_refuse_instead_of_changing_membership() {
    let (base, worker) = crate::native::remote::gateway_tests::server(vec![(200, "ok")]);
    let remote = RemoteBackend::connect(&base, GRAPH, None, None, &[]).unwrap();
    worker.join().unwrap();
    for (predicate, competing, filter) in [
        (
            term::assigned_to(),
            Value::Str("other".into()),
            Filter {
                assignee: Some("other".into()),
                ..Default::default()
            },
        ),
        (
            term::status(),
            Value::Str("closed".into()),
            Filter {
                status: Some("closed".into()),
                ..Default::default()
            },
        ),
        (
            term::priority(),
            Value::Int(2),
            Filter {
                priority: Some("P2".into()),
                ..Default::default()
            },
        ),
    ] {
        let mut private = Store::open_in_memory().unwrap();
        let mut values = fields(Some("open"), Some(1), Some("reviewer"));
        values.push((predicate, competing));
        seed(
            &mut private,
            GRAPH,
            "urn:count:ambiguous",
            "ambiguous",
            values,
        );
        let req = CountReq {
            filter,
            by: None,
            include_closed: false,
        };
        let actual = decode_count(
            rows(&private, &remote.count_query(&req).unwrap()),
            false,
            None,
        );
        assert!(
            actual.is_err(),
            "competing scalar must not silently pick ANY matching fact: {req:?}"
        );
    }
}
