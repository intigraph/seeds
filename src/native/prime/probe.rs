//! Read-only worker. The parent kills/reaps this process at its deadline.
use super::super::{config, Cli};
use crate::backend::{Backend, Ctx};
use crate::engine::{self, Filter, ListReq, ReadyReq};
use crate::error::{ErrorKind, Result, SdError};
use serde_json::{json, Value};

pub(super) fn empty_summary(at: Option<u64>) -> Value {
    json!({"board": null, "reachable": null, "as_of_tx": at, "graph": null, "actor": null, "plate": null, "plate_total": null, "ready_count": null})
}

pub(super) fn run(cli: &Cli) -> Value {
    let mut scope = empty_summary(cli.at);
    match read(cli, &mut scope) {
        Ok(()) => json!({"status": "ok", "summary": scope}),
        Err(e) => {
            if e.kind == ErrorKind::Unreachable {
                scope["reachable"] = json!(false);
            }
            json!({"status": if e.kind == ErrorKind::NotFound { "absent" } else if e.kind == ErrorKind::Unreachable { "unreachable" } else { "unreadable" }, "summary": scope,
            "reason": format!("{:?}", e.kind), "hint": "Run `sd where --json` and `sd doctor`; configure a board or initialize one deliberately with `sd init`."})
        }
    }
}

fn read(cli: &Cli, scope: &mut Value) -> Result<()> {
    let cfg = config::Inputs::from_env(cli.store.clone(), cli.quipu.clone(), cli.graph.clone())
        .and_then(|i| config::resolve(&i))?;
    let owner = cli
        .actor
        .clone()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::env::var("SHANTY_AGENT").ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| super::super::actor(cli));
    scope["actor"] = json!(owner);
    scope["board"] = match &cfg.location {
        config::Location::Store(path) => json!(path),
        config::Location::Url(raw) => {
            let mut url = url::Url::parse(raw).map_err(|_| SdError::usage("invalid board URL"))?;
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            json!(url.as_str())
        }
    };
    // Never invent a graph/project id, import a pendant, enroll or write.
    let graph = config::project_graph(&cfg, false)?.0.ok_or_else(|| {
        let existing = matches!(&cfg.location, config::Location::Store(path) if path.exists());
        SdError::new(
            if existing {
                ErrorKind::Config
            } else {
                ErrorKind::NotFound
            },
            "no project identity",
        )
    })?;
    scope["graph"] = json!(graph);
    scope["scope"] =
        json!("configured board at read time; local working store only, no pendant import or sync");
    let ctx = Ctx {
        now: super::super::now(),
        actor: owner,
        prefix: cfg.prefix.clone(),
        claims: Default::default(),
    };
    match &cfg.location {
        config::Location::Url(url) => {
            let remote = super::super::remote::RemoteBackend::connect(
                url,
                &graph,
                super::super::token(&cfg, url, cfg.location_from_project)?,
                super::super::signer(&cfg, url, cfg.location_from_project)?,
                &cfg.allow_plain_http_hosts,
            )?;
            scope["reachable"] = json!(true);
            scope["replica"] = remote.replica().map_or(Value::Null, |r| r.to_json());
            board(&remote, &ctx, cli.at, scope)
        }
        config::Location::Store(path) => {
            let canonical = std::fs::canonicalize(path).map_err(|e| {
                SdError::new(
                    if e.kind() == std::io::ErrorKind::NotFound {
                        ErrorKind::NotFound
                    } else {
                        ErrorKind::Failed
                    },
                    "cannot read local store",
                )
            })?;
            let mut wal = canonical.as_os_str().to_os_string();
            wal.push("-wal");
            let name = canonical
                .to_str()
                .ok_or_else(|| SdError::failed("non-UTF8 local store path"))?;
            let store = if matches!(std::fs::metadata(wal), Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
                quipu::Store::open_immutable(name)
            } else {
                quipu::Store::open_read_only(name)
            }.map_err(|_| SdError::failed("cannot open local store read-only"))?;
            scope["reachable"] = json!(true);
            board(
                &crate::quipu_backend::QuipuBackend::read_only(store, &graph),
                &ctx,
                cli.at,
                scope,
            )
        }
    }
}

fn board(backend: &dyn Backend, ctx: &Ctx, at: Option<u64>, scope: &mut Value) -> Result<()> {
    // Historical engine reads currently require a whole snapshot. Killing the
    // client does not cancel that work on the server; leave this summary unknown.
    if at.is_some() && backend.scoped_reads() {
        return Err(SdError::refused(
            "historical remote prime summary unavailable; use explicit sd list/ready --at reads",
        ));
    }
    super::super::refuse_old_vocabulary("prime board", backend)?;
    let page = engine::list(
        backend,
        &ListReq {
            filter: Filter {
                assignee: Some(ctx.actor.clone()),
                ..Default::default()
            },
            limit: Some(10),
            deferred: true,
            ..Default::default()
        },
        at,
    )?;
    scope["plate_total"] = json!(page.total);
    scope["plate"] = json!(page.issues.iter().take(10).map(|s| json!({
        "id": s.id, "title": s.title.chars().take(160).collect::<String>(), "status": s.status, "priority": s.priority
    })).collect::<Vec<_>>());
    let ready = engine::ready(
        backend,
        ctx,
        &ReadyReq {
            // A page limit only trims the response; bind the actor before
            // readiness is computed so remote work never scans the board.
            filter: Filter {
                assignee: Some(ctx.actor.clone()),
                ..Default::default()
            },
            limit: Some(1),
            ..Default::default()
        },
        at,
    )?;
    scope["ready_count"] = json!(ready.total);
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::backend::{Claims, SeedQuery, WriteBatch};
    use crate::model::Snapshot;
    use std::cell::Cell;
    use std::collections::BTreeMap;

    struct CountingBackend {
        fixture: Snapshot,
        ready_reads: Cell<usize>,
        filtered_reads: Cell<usize>,
    }

    impl Backend for CountingBackend {
        fn scoped_reads(&self) -> bool {
            true
        }
        fn snapshot(&self, _: Option<u64>) -> Result<Snapshot> {
            panic!("prime hydrated the whole board")
        }
        fn ready_ids(&self, _: Option<u64>) -> Result<Vec<String>> {
            panic!("prime requested board-wide readiness")
        }
        fn ready_ids_where(&self, query: &SeedQuery) -> Result<Option<Vec<String>>> {
            self.ready_reads.set(self.ready_reads.get() + 1);
            assert_eq!(query.assignee.as_deref(), Some("fixture-agent"));
            Ok(Some(
                self.fixture
                    .seeds
                    .values()
                    .filter(|seed| query.matches(seed) && seed.status == "open")
                    .map(|seed| seed.id.clone())
                    .collect(),
            ))
        }
        fn snapshot_where(&self, query: &SeedQuery) -> Result<Snapshot> {
            self.filtered_reads.set(self.filtered_reads.get() + 1);
            assert_eq!(query.assignee.as_deref(), Some("fixture-agent"));
            let mut snap = self.fixture.clone();
            snap.seeds.retain(|_, seed| query.matches(seed));
            Ok(snap)
        }
        fn snapshot_seeds(&self, ids: &[String]) -> Result<Snapshot> {
            let mut snap = self.fixture.clone();
            for id in ids {
                assert_eq!(snap.seeds[id].assignee.as_deref(), Some("fixture-agent"));
            }
            snap.seeds.retain(|id, _| ids.contains(id));
            Ok(snap)
        }
        fn dependent_counts(&self, _: &[String]) -> Result<Option<BTreeMap<String, usize>>> {
            Ok(Some(BTreeMap::new()))
        }
        fn commit(&mut self, _: &WriteBatch, _: &Ctx) -> Result<u64> {
            panic!("prime attempted a write")
        }
        fn claims_of(&self, _: &str) -> Result<Vec<(u64, Claims)>> {
            unreachable!()
        }
    }

    #[test]
    fn prime_ready_count_is_actor_scoped_before_backend_reads() {
        let ctx = Ctx {
            now: "2026-10-01T00:00:00Z".into(),
            actor: "fixture-agent".into(),
            prefix: "sd".into(),
            claims: Default::default(),
        };
        let mut fixture =
            crate::quipu_backend::QuipuBackend::in_memory("https://example.test/project").unwrap();
        for owner in ["fixture-agent", "fixture-agent", "somebody-else", ""] {
            engine::create(
                &mut fixture,
                &ctx,
                &engine::CreateReq {
                    title: format!("Item {owner}"),
                    assignee: Some(owner.into()),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let backend = CountingBackend {
            fixture: fixture.snapshot(None).unwrap(),
            ready_reads: Cell::new(0),
            filtered_reads: Cell::new(0),
        };
        let mut summary = empty_summary(None);
        board(&backend, &ctx, None, &mut summary).unwrap();
        assert_eq!(summary["plate_total"], 2);
        assert_eq!(summary["ready_count"], 2, "total, not the one-row page");
        assert_eq!(backend.ready_reads.get(), 1);
        assert!(
            backend.filtered_reads.get() >= 2,
            "plate and deferred ready scope"
        );
        let prior = (backend.ready_reads.get(), backend.filtered_reads.get());
        let mut pinned = empty_summary(Some(1));
        assert_eq!(
            board(&backend, &ctx, Some(1), &mut pinned)
                .unwrap_err()
                .kind,
            ErrorKind::Refused
        );
        assert_eq!(
            (backend.ready_reads.get(), backend.filtered_reads.get()),
            prior
        );
        assert!(pinned["ready_count"].is_null());
    }
}
