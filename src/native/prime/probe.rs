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
            limit: Some(1),
            ..Default::default()
        },
        at,
    )?;
    scope["ready_count"] = json!(ready.total);
    Ok(())
}
