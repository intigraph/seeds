//! aegis-az3d.1 phase 0: local measurement of projection query latency.
//! Ignored by default; run explicitly. No network, no production state.
#![cfg(feature = "native")]
#![allow(clippy::unwrap_used, clippy::disallowed_methods, clippy::print_stdout)]

use quipu::sparql::{query_temporal, QueryResult, TemporalContext};
use quipu::Store;
use seeds::backend::Ctx;
use seeds::engine::{self, BulkItem, CreateReq, UpdateReq};
use seeds::quipu_backend::QuipuBackend;
use seeds::vocab;
use std::time::Instant;

const GRAPH: &str = "https://seeds.local/project/aegis";
const AGENTS: &[&str] = &[
    "malcolm", "wu", "sattler", "muldoon", "ian", "arnold", "franklin", "kelly", "ellie",
    "hammond", "dearing", "harding", "gennaro", "nedry", "grant", "lex", "tim", "henry",
    "zara", "owen", "claire", "barry", "lowery", "vivian", "ray", "donald", "robert", "sarah",
    "nick", "eddie", "dieter", "ajay", "roland", "peter", "billy", "udesky", "amanda", "paul",
    "eric", "maisie",
];

fn ctx() -> Ctx {
    Ctx {
        now: "2026-11-26T00:00:00Z".into(),
        actor: "fixture".into(),
        prefix: "aegis".into(),
        claims: Default::default(),
    }
}

fn env(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Build a board through the real seeds engine: live items in mixed states,
/// closed items that went open -> (in_progress) -> closed, so the closed ones
/// carry retracted historical status rows exactly as production does.
fn build(store: Store, live: usize, closed: usize) -> QuipuBackend {
    let mut b = QuipuBackend::writable(store, GRAPH).unwrap();
    b.store().graph_create(&vocab::ephemeral_graph(GRAPH)).unwrap();
    let total = live + closed;
    let batch = env("AZ_BATCH", 500);
    let t0 = Instant::now();
    let mut ids = Vec::with_capacity(total);
    let mut i = 0;
    while i < total {
        let n = batch.min(total - i);
        let items: Vec<BulkItem> = (i..i + n)
            .map(|k| BulkItem {
                req: CreateReq {
                    title: format!("fixture item {k} lorem ipsum dolor sit amet"),
                    description: Some(format!(
                        "Synthetic description {k}: consectetur adipiscing elit sed do eiusmod"
                    )),
                    priority: Some(format!("{}", k % 5)),
                    issue_type: Some(if k % 7 == 0 { "bug".into() } else { "task".into() }),
                    assignee: Some(AGENTS[k % AGENTS.len()].into()),
                    labels: vec![format!("label{}", k % 13)],
                    acceptance_criteria: Some("criteria".into()),
                    ..Default::default()
                },
                notes: Some(format!("notes {k}")),
                design: None,
            })
            .collect();
        let (seeds, _) = engine::create_many(&mut b, &ctx(), &items, false).unwrap();
        ids.extend(seeds.into_iter().map(|s| s.id));
        i += n;
    }
    eprintln!("created {total} in {:?}", t0.elapsed());
    let t1 = Instant::now();
    // Closed: every third goes through in_progress first.
    let closed_ids = &ids[..closed];
    for chunk in closed_ids.chunks(batch) {
        let prog: Vec<String> = chunk.iter().step_by(3).cloned().collect();
        engine::update(
            &mut b,
            &ctx(),
            &prog,
            &UpdateReq {
                status: Some("in_progress".into()),
                ..Default::default()
            },
        )
        .unwrap();
        engine::close(&mut b, &ctx(), chunk, Some("done fixture"), true).unwrap();
    }
    eprintln!("closed {closed} in {:?}", t1.elapsed());
    let t2 = Instant::now();
    // Live: 50% open, 25% in_progress, 15% blocked, 10% hooked.
    let live_ids = &ids[closed..];
    for (state, pick) in [("in_progress", 0usize), ("blocked", 1), ("hooked", 2)] {
        let sel: Vec<String> = live_ids
            .iter()
            .enumerate()
            .filter(|(k, _)| match pick {
                0 => k % 4 == 0,
                1 => k % 20 == 1 || k % 20 == 5 || k % 20 == 9,
                _ => k % 10 == 3,
            })
            .map(|(_, id)| id.clone())
            .collect();
        for chunk in sel.chunks(batch) {
            engine::update(
                &mut b,
                &ctx(),
                chunk,
                &UpdateReq {
                    status: Some(state.into()),
                    ..Default::default()
                },
            )
            .unwrap();
        }
    }
    eprintln!("live states in {:?}", t2.elapsed());
    b
}

fn run(store: &Store, q: &str, as_of: Option<i64>) -> (f64, usize) {
    let ctx = TemporalContext {
        as_of_tx: as_of,
        deadline: Some(quipu::time::Deadline::after_millis(600_000)),
        row_cap: Some(10_000),
        ..Default::default()
    };
    let t = Instant::now();
    let n = match query_temporal(store, q, &ctx).unwrap() {
        QueryResult::Ask(v) => usize::from(v),
        QueryResult::Select { rows, .. } => rows.len(),
        _ => 0,
    };
    (t.elapsed().as_secs_f64() * 1000.0, n)
}

fn time5(label: &str, store: &Store, q: &str, as_of: Option<i64>) {
    let mut v: Vec<f64> = Vec::new();
    let mut rows = 0;
    for _ in 0..env("AZ_REPS", 5) {
        let (ms, n) = run(store, q, as_of);
        v.push(ms);
        rows = n;
    }
    v.sort_by(f64::total_cmp);
    println!(
        "RESULT|{label}|as_of={}|rows={rows}|median_ms={:.1}|max_ms={:.1}|all={:?}",
        as_of.map_or("none".into(), |t| t.to_string()),
        v[v.len() / 2],
        v[v.len() - 1],
        v.iter().map(|x| (x * 10.0).round() / 10.0).collect::<Vec<_>>()
    );
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
#[ignore = "az3d.1 phase-0 benchmark; run explicitly"]
fn az3d1_phase0_bench() {
    let live = env("AZ_LIVE", 6000);
    let closed = env("AZ_CLOSED", 115_000);
    let dir = std::env::var("AZ_DIR").unwrap();
    let store = match std::env::var("AZ_FILE") {
        Ok(p) => {
            let _ = std::fs::remove_file(&p);
            Store::open(&p).unwrap()
        }
        Err(_) => Store::open_in_memory().unwrap(),
    };
    let t = Instant::now();
    if let Ok(path) = std::env::var("AZ_OPEN") {
        drop(store);
        if std::env::var("AZ_PTRACER").is_ok() {
            // Allow a sampling debugger to attach (yama ptrace_scope=1).
            unsafe { libc::prctl(libc::PR_SET_PTRACER, libc::PR_SET_PTRACER_ANY, 0, 0, 0) };
            println!("PID|{}", std::process::id());
        }
        let s = Store::open(&path).unwrap();
        let head = s.transaction_head().unwrap();
        let q = read(&format!("{dir}/{}.rq", std::env::var("AZ_LOOP").unwrap()));
        let as_of = std::env::var("AZ_NOASOF").is_err().then_some(head);
        for _ in 0..env("AZ_N", 1) {
            let (ms, n) = run(&s, &q, as_of);
            println!("LOOP|ms={ms:.2}|rows={n}");
        }
        return;
    }
    let b = build(store, live, closed);
    let store = b.store();
    let head = store.transaction_head().unwrap();
    println!("FIXTURE|live={live}|closed={closed}|head_tx={head}|build_s={:.1}", t.elapsed().as_secs_f64());
    let count = run(
        store,
        &format!("SELECT (COUNT(*) AS ?n) WHERE {{ GRAPH <{GRAPH}> {{ ?s ?p ?o }} }}"),
        None,
    );
    let _ = count;
    let cq = format!("SELECT (COUNT(*) AS ?n) WHERE {{ GRAPH <{GRAPH}> {{ ?s ?p ?o }} }}");
    if let QueryResult::Select { rows, .. } =
        query_temporal(store, &cq, &TemporalContext::default()).unwrap()
    {
        println!("FIXTURE|current_triples={:?}", rows[0].get("n").map(|v| quipu::value_to_json(store, v)));
    }
    if std::env::var("AZ_DUMP").is_ok() {
        let dq = format!(
            "SELECT ?p ?o (DATATYPE(?o) AS ?dt) WHERE {{ GRAPH <{GRAPH}> {{ ?s ?p ?o . ?s <https://schema.org/agent> ?a }} }} LIMIT 60"
        );
        if let QueryResult::Select { rows, .. } =
            query_temporal(store, &dq, &TemporalContext::default()).unwrap()
        {
            for r in rows {
                let m: Vec<String> = r
                    .iter()
                    .map(|(k, v)| format!("{k}={}", quipu::value_to_json(store, v)))
                    .collect();
                println!("DUMP|{}", m.join(" "));
            }
        }
    }
    let queries = [
        "one_triple_literal",
        "one_triple_iri",
        "one_triple_status_open",
        "one_triple_two_graph",
        "heavy",
        "heavy_single_graph",
        "control",
    ];
    for name in queries {
        let p = format!("{dir}/{name}.rq");
        if !std::path::Path::new(&p).exists() {
            continue;
        }
        let q = read(&p);
        time5(name, store, &q, Some(head));
        time5(name, store, &q, None);
    }
    // (e) production rhythm: a small write lands, then the query runs.
    let one = read(&format!("{dir}/one_triple_literal.rq"));
    let heavy = read(&format!("{dir}/heavy.rq"));
    let mut b = b;
    let mut after = Vec::new();
    let mut after_h = Vec::new();
    for k in 0..5 {
        engine::create(
            &mut b,
            &ctx(),
            &CreateReq {
                title: format!("late {k}"),
                assignee: Some("wu".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let h = b.store().transaction_head().unwrap();
        after.push(run(b.store(), &one, Some(h)).0);
        engine::create(
            &mut b,
            &ctx(),
            &CreateReq {
                title: format!("late2 {k}"),
                ..Default::default()
            },
        )
        .unwrap();
        let h = b.store().transaction_head().unwrap();
        after_h.push(run(b.store(), &heavy, Some(h)).0);
    }
    println!("RESULT|one_triple_after_write|as_of=head|all={after:?}");
    println!("RESULT|heavy_after_write|as_of=head|all={after_h:?}");
}
