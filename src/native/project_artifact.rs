//! Repository project bundles use Quipu's share/load gates, never soft resealing.
use std::path::Path;

use crate::backend::Ctx;
use crate::error::{Result, SdError};
use crate::pendant::{self, Pendant, Seal};
use crate::quipu_backend::QuipuBackend;
use quipu::project_graph as project;

use super::{cli::Cli, config, store, Outcome};

// Explicit recipient policy pin: this profile is unchanged from the published
// Seeds release, not a policy selected by the incoming share.
const PROFILE_SHA256: &str =
    "sha256:b1b3bf0846298e437da15f89d3fc599ee41663f9258c3d40b03f033804d6a04f";
const PROFILE_REF: &str = "seeds-ai-v0.22.1@30e17964c5a9f82b25e61614ed3fa71ada321901";

fn require_profile(b: &QuipuBackend) -> Result<()> {
    if b.store()
        .list_shapes()?
        .iter()
        .any(|(_, body, _)| pendant::sha256(body.as_bytes()) == PROFILE_SHA256)
    {
        return Ok(());
    }
    Err(SdError::refused(format!("recipient needs the reviewed Seeds profile {PROFILE_REF} ({PROFILE_SHA256}); verify its digest, then run `quipu shapes load seeds shapes/seeds.shapes.ttl --db <recipient.db>` before `quipu load`; no bundle policy was installed")))
}

pub(super) fn is_project_dir(dir: &Path) -> bool {
    dir.file_name().is_some_and(|n| n == "graph")
        && dir
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == ".quipu")
}

fn root(dir: &Path) -> Result<&Path> {
    dir.parent()
        .and_then(Path::parent)
        .ok_or_else(|| SdError::refused("invalid project bundle path"))
}

fn check_target(b: &QuipuBackend, dir: &Path) -> Result<()> {
    let base = dir
        .parent()
        .ok_or_else(|| SdError::refused("missing .quipu directory"))?;
    for path in [base.to_path_buf(), base.join("project")] {
        if std::fs::symlink_metadata(&path)
            .map_err(|e| SdError::refused(e.to_string()))?
            .file_type()
            .is_symlink()
        {
            return Err(SdError::refused(
                "project identity symlinks are not supported",
            ));
        }
    }
    let id = project::read_project_id(root(dir)?)?
        .ok_or_else(|| SdError::refused("project bundle needs .quipu/project"))?;
    if project::project_iri(&id) != b.graph_iri() {
        return Err(SdError::refused(
            "configured graph differs from .quipu/project; nothing exported",
        ));
    }
    Ok(())
}

fn sealed(dir: &Path) -> Result<Pendant> {
    // Repository inputs may be untrusted: never follow a bundle symlink into
    // another project, including a member swapped out for a local file.
    for path in std::iter::once(dir.to_path_buf()).chain(pendant::FILES.iter().map(|n| dir.join(n)))
    {
        if std::fs::symlink_metadata(&path)
            .map_err(|e| SdError::refused(e.to_string()))?
            .file_type()
            .is_symlink()
        {
            return Err(SdError::refused(
                "project bundle symlinks are not supported",
            ));
        }
    }
    let p = store::read_pendant_dir(dir)?
        .ok_or_else(|| SdError::refused("project bundle is missing export.nt"))?;
    if pendant::seal(&p)? != Seal::Intact {
        return Err(SdError::refused(
            "project bundle seal is broken; restore or explicitly re-share it with Quipu",
        ));
    }
    Ok(p)
}

/// Same ordinary import and named-graph promotion used by `quipu load`.
fn load(b: &mut QuipuBackend, dir: &Path, ctx: &Ctx, expected_scope: &str) -> Result<()> {
    sealed(dir)?;
    let reference = dir
        .to_str()
        .ok_or_else(|| SdError::refused("non-UTF8 project bundle path"))?;
    let request = quipu::share_transport::read_reference(reference)?;
    load_request(b, request, ctx, expected_scope)
}

fn load_request(
    b: &mut QuipuBackend,
    mut request: quipu::share_import::ShareImportRequest,
    ctx: &Ctx,
    expected_scope: &str,
) -> Result<()> {
    require_profile(b)?;
    if !matches!(&request.manifest.scope, quipu::share::ShareScope::Graph(g) if g == expected_scope)
    {
        return Err(SdError::refused(
            "bundle scope differs from the expected project graph",
        ));
    }
    // This is a local repository import, as `quipu load --destination internal`.
    // Internal skips the outward identifier scrub, not hash/shape/quarantine gates.
    request.destination = quipu::share_scrub::ShareDestination::Internal;
    let target = b.graph_iri().to_owned();
    let imported =
        quipu::share_import::import_share(b.store_mut(), &request, &ctx.now, Some(&ctx.actor))?;
    if !imported.promotion.eligible {
        return Err(SdError::refused(format!(
            "project bundle quarantined: {:?}",
            imported.promotion.blockers
        )));
    }
    project::promote_into_graph(
        b.store_mut(),
        &imported.share_id,
        &imported.staging_graph,
        &target,
        &ctx.now,
        Some(&ctx.actor),
    )?;
    Ok(())
}

fn allow_config(root: &Path) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(root.join(".quipu/.gitignore"))
        .map_err(|e| SdError::failed(e.to_string()))?;
    file.write_all(b"\n!config.toml\n")
        .and_then(|()| file.sync_all())
        .map_err(|e| SdError::failed(e.to_string()))
}

pub(super) fn export(b: &QuipuBackend, path: &Path, dir: &Path) -> Result<bool> {
    check_target(b, dir)?;
    let p = pendant::export(b)?;
    if dir.exists() {
        let disk = sealed(dir)?;
        if disk.export_nt() == p.export_nt()
            && disk.files.get(pendant::SHAPES_TTL) == p.files.get(pendant::SHAPES_TTL)
        {
            store::write_marker(path, &p.export_hash().unwrap_or_default())?;
            return Ok(false);
        }
    }
    let root = root(dir)?;
    // Quipu's allow-list and atomic bundle swap; partial exports stay ignored.
    project::scaffold(root, None)?;
    if root.join(".quipu/config.toml").is_file() {
        allow_config(root)?;
    }
    let next = project::clear_next_bundle(root)?;
    let opts = quipu::share::ShareOptions {
        scope: quipu::share::ShareScope::Graph(b.graph_iri().to_owned()),
        shapes: vec![crate::quipu_backend::SHAPES_NAME.to_owned()],
        destination: quipu::share::ShareDestination::Internal,
        ..Default::default()
    };
    quipu::share::share(
        b.store(),
        next.to_str()
            .ok_or_else(|| SdError::refused("non-UTF8 project path"))?,
        &opts,
    )?;
    sealed(&next)?;
    project::install_next_bundle(root)?;
    store::write_marker(path, &p.export_hash().unwrap_or_default())?;
    Ok(true)
}

pub(super) fn hydrate(
    b: &mut QuipuBackend,
    path: &Path,
    dir: &Path,
    ctx: &Ctx,
) -> Result<Vec<String>> {
    check_target(b, dir)?;
    let mine = pendant::export(b)?;
    if !dir.exists() {
        return Err(SdError::refused(
            "project bundle missing; explicitly share or migrate before using it",
        ));
    }
    let disk = sealed(dir)?;
    let mine_hash = mine.export_hash().unwrap_or_default();
    let disk_hash = disk.export_hash().unwrap_or_default();
    if mine_hash == disk_hash {
        store::write_marker(path, &disk_hash)?;
        return Ok(Vec::new());
    }
    let marker = store::read_marker(path);
    let empty = b
        .store()
        .lookup(b.graph_iri())?
        .map(|g| b.store().current_facts_in_graph(g).map(|f| f.is_empty()))
        .transpose()?
        .unwrap_or(true);
    if empty || marker.as_deref() == Some(mine_hash.as_str()) {
        let graph = b.graph_iri().to_owned();
        load(b, dir, ctx, &graph)?;
        if pendant::export(b)?.export_nt() != disk.export_nt() {
            return Err(SdError::refused(
                "project load changed the exported facts; inspect staged import",
            ));
        }
        store::write_marker(path, &disk_hash)?;
        return Ok(vec!["loaded verified Quipu project bundle".into()]);
    }
    if marker.as_deref() == Some(disk_hash.as_str()) {
        export(b, path, dir)?;
        return Ok(vec![
            "exported working changes to Quipu project bundle".into()
        ]);
    }
    Err(SdError::conflict(
        "working store and project bundle diverged; nothing reconciled",
    ))
}

/// Explicit local-only migration. The legacy bundle stays intact as rollback;
/// only after complete export equality does the config select the new bundle.
pub(super) fn migrate(cli: &Cli) -> Result<Outcome> {
    let inputs = config::Inputs::from_env(cli.store.clone(), cli.quipu.clone(), cli.graph.clone())?;
    if cli.at.is_some()
        || inputs.flag_store.is_some()
        || inputs.flag_url.is_some()
        || inputs.flag_graph.is_some()
        || inputs.env_store.is_some()
        || inputs.env_url.is_some()
        || inputs.env_graph.is_some()
        || inputs.env_pendant.is_some()
    {
        return Err(SdError::refused("migrate-project requires the repository's local configuration, without destination overrides"));
    }
    let cfg = config::resolve(&inputs)?;
    let config::Location::Store(path) = &cfg.location else {
        return Err(SdError::refused(
            "migrate-project never migrates a remote board",
        ));
    };
    let root = config::find_project_root(&inputs.cwd)
        .ok_or_else(|| SdError::refused("no legacy project"))?;
    if std::fs::symlink_metadata(root.join(".quipu")).is_ok() {
        return Err(SdError::refused(
            ".quipu already exists; refusing ambiguous migration destination",
        ));
    }
    for path in [
        root.join(".seeds"),
        root.join(".seeds/config.toml"),
        cfg.project_id_file.clone(),
    ] {
        if std::fs::symlink_metadata(path)
            .map_err(|e| SdError::refused(e.to_string()))?
            .file_type()
            .is_symlink()
        {
            return Err(SdError::refused(
                "legacy project symlinks must be reconciled before migration",
            ));
        }
    }
    let legacy = root.join(".seeds/pendant");
    if cfg.pendant.as_deref() != Some(legacy.as_path()) {
        return Err(SdError::refused(
            "migration supports only the configured .seeds/pendant ledger",
        ));
    }
    let original = sealed(&legacy)?;
    let graph = config::project_graph(&cfg, false)?
        .0
        .ok_or_else(|| SdError::refused("missing legacy project id"))?;
    if let Some(existing) = store::open_for_read(path, &graph)? {
        if pendant::export(&existing)?.export_nt() != original.export_nt() {
            return Err(SdError::conflict(
                "legacy working store differs from its sealed artifact; reconcile before migration",
            ));
        }
    }
    let id = std::fs::read_to_string(&cfg.project_id_file)
        .map_err(|e| SdError::failed(e.to_string()))?;
    let id = id.trim();
    project::validate_project_id(id)?;
    let mut backend = QuipuBackend::in_memory(&project::project_iri(id))?;
    let ctx = Ctx {
        now: super::now(),
        actor: super::actor(cli),
        prefix: cfg.prefix.clone(),
        claims: super::claims(cli)?,
    };
    load(&mut backend, &legacy, &ctx, &graph)?;
    if pendant::export(&backend)?.export_nt() != original.export_nt() {
        return Err(SdError::refused(
            "migration changed facts; no project files written",
        ));
    }
    let config_path = root.join(".seeds/config.toml");
    let config_text =
        std::fs::read_to_string(&config_path).map_err(|e| SdError::failed(e.to_string()))?;
    let mut document: toml::Value =
        toml::from_str(&config_text).map_err(|e| SdError::refused(e.to_string()))?;
    if document
        .get("project")
        .and_then(|p| p.get("graph"))
        .is_some()
    {
        return Err(SdError::refused(
            "explicit graph configuration must be reconciled before migration",
        ));
    }
    document["pendant"]["dir"] = toml::Value::String(".quipu/graph".into());
    let config_next =
        toml::to_string_pretty(&document).map_err(|e| SdError::failed(e.to_string()))?;
    publish_migration(&root, id, &backend, &original, &config_next, |_| Ok(()))?;
    Ok(super::ok(
        cli.wants_json(),
        serde_json::json!({"status":"ok", "graph":backend.graph_iri(), "export_hash":original.export_hash(), "legacy_retained":true}),
        "migrated verified ledger to .quipu/graph; legacy bundle retained for rollback".into(),
        Vec::new(),
    ))
}

// The reserved destination stays empty until the only commit: renaming the
// complete directory over it. Neither resolver nor plain Quipu can discover
// the staged identity. Every fallible write happens before that rename.
fn publish_migration(
    root: &Path,
    id: &str,
    backend: &QuipuBackend,
    original: &Pendant,
    config_next: &str,
    after_scaffold: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    struct Stage {
        root: std::path::PathBuf,
        destination: std::path::PathBuf,
        reserved: bool,
    }
    impl Drop for Stage {
        fn drop(&mut self) {
            // Only our exclusively created staging tree; never another writer's.
            let _ = std::fs::remove_dir_all(&self.root);
            // Removes only an empty reservation, never a committed directory.
            if self.reserved {
                let _ = std::fs::remove_dir(&self.destination);
            }
        }
    }
    let stage_root = root.join(".seeds/project-migration-next");
    std::fs::create_dir(&stage_root)
        .map_err(|e| SdError::refused(format!("cannot reserve migration stage: {e}")))?;
    let mut stage = Stage {
        root: stage_root,
        destination: root.join(".quipu"),
        reserved: false,
    };
    std::fs::create_dir(&stage.destination)
        .map_err(|e| SdError::refused(format!("cannot reserve migration destination: {e}")))?;
    stage.reserved = true;
    project::scaffold(&stage.root, Some(id))?;
    let dir = project::graph_dir(&stage.root);
    export(backend, &stage.root.join(".quipu/local.db"), &dir)?;
    if sealed(&dir)?.export_nt() != original.export_nt() {
        return Err(SdError::refused(
            "migration read-back differs; legacy configuration retained",
        ));
    }
    after_scaffold(&stage.root)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.root.join(".quipu/config.toml"))
        .map_err(|e| SdError::refused(format!("cannot stage migrated configuration: {e}")))?;
    file.write_all(config_next.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| SdError::failed(e.to_string()))?;
    // Retain Quipu's database/key exclusions, adding only the Seeds config.
    allow_config(&stage.root)?;
    std::fs::rename(stage.root.join(".quipu"), &stage.destination)
        .map_err(|e| SdError::failed(format!("cannot commit migrated project: {e}")))?;
    Ok(())
}

/// Verified read-only preview for transfer: no disk/store/marker writes.
pub(super) fn preview(
    b: &QuipuBackend,
    path: &Path,
    dir: &Path,
    ctx: &Ctx,
) -> Result<QuipuBackend> {
    check_target(b, dir)?;
    let mine = pendant::export(b)?;
    let disk = sealed(dir)?;
    let mine_hash = mine.export_hash().unwrap_or_default();
    let disk_hash = disk.export_hash().unwrap_or_default();
    let marker = store::read_marker(path);
    let empty = b
        .store()
        .lookup(b.graph_iri())?
        .map(|g| b.store().current_facts_in_graph(g).map(|f| f.is_empty()))
        .transpose()?
        .unwrap_or(true);
    let chosen = if mine_hash == disk_hash || empty || marker.as_deref() == Some(mine_hash.as_str())
    {
        disk
    } else if marker.as_deref() == Some(disk_hash.as_str()) {
        mine
    } else {
        return Err(SdError::conflict(
            "working store and project bundle diverged; no preview created",
        ));
    };
    let request = serde_json::from_value(serde_json::json!({
        "manifest": serde_json::from_str::<serde_json::Value>(&chosen.files[pendant::MANIFEST_JSON])
            .map_err(|e| SdError::refused(e.to_string()))?,
        "export_ntriples": chosen.files[pendant::EXPORT_NT],
        "shapes_turtle": chosen.files[pendant::SHAPES_TTL],
        "source": "seeds:project-preview"
    }))
    .map_err(|e| SdError::refused(e.to_string()))?;
    let mut preview = QuipuBackend::in_memory(b.graph_iri())?;
    load_request(&mut preview, request, ctx, b.graph_iri())?;
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postscaffold_failure_never_publishes_identity_or_changes_resolution() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir(root.join(".seeds")).unwrap();
        let old_config = "[pendant]\ndir = \".seeds/pendant\"\n";
        std::fs::write(root.join(".seeds/config.toml"), old_config).unwrap();
        std::fs::write(root.join(".seeds/project-id"), "atomic1\n").unwrap();
        let inputs = config::Inputs {
            cwd: root.to_owned(),
            ..Default::default()
        };
        let before = config::resolve(&inputs).unwrap();
        let before_graph = config::project_graph(&before, false).unwrap().0;
        let backend = QuipuBackend::in_memory(&project::project_iri("atomic1")).unwrap();
        let original = pendant::export(&backend).unwrap();
        let failure = publish_migration(
            root,
            "atomic1",
            &backend,
            &original,
            "[pendant]\ndir = \".quipu/graph\"\n",
            |stage| {
                assert!(stage.join(".quipu/project").is_file());
                assert!(stage.join(".quipu/graph/export.nt").is_file());
                assert!(project::read_project_id(root)?.is_none());
                let during = config::resolve(&inputs)?;
                assert_eq!(during.location, before.location);
                assert_eq!(config::project_graph(&during, false)?.0, before_graph);
                // A genuine late filesystem failure, after complete scaffolding.
                std::fs::create_dir(stage.join(".quipu/config.toml")).unwrap();
                Ok(())
            },
        )
        .unwrap_err();
        assert!(failure
            .message
            .contains("cannot stage migrated configuration"));
        assert!(!root.join(".quipu").exists());
        assert!(!root.join(".seeds/project-migration-next").exists());
        assert_eq!(
            std::fs::read_to_string(root.join(".seeds/config.toml")).unwrap(),
            old_config
        );
        let after = config::resolve(&inputs).unwrap();
        assert_eq!(after.location, before.location);
        assert_eq!(after.pendant, before.pendant);
        assert_eq!(
            config::project_graph(&after, false).unwrap().0,
            before_graph
        );
        // Retry proves the positive arm commits all three together.
        publish_migration(
            root,
            "atomic1",
            &backend,
            &original,
            "[pendant]\ndir = \".quipu/graph\"\n",
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            project::read_project_id(root).unwrap().as_deref(),
            Some("atomic1")
        );
        let after = config::resolve(&inputs).unwrap();
        assert_eq!(
            after.location,
            config::Location::Store(root.join(".quipu/local.db"))
        );
        assert_eq!(after.pendant, Some(root.join(".quipu/graph")));
        assert_eq!(
            std::fs::read_to_string(root.join(".seeds/config.toml")).unwrap(),
            old_config
        );
    }

    #[test]
    fn missing_or_different_recipient_profile_refuses_before_import_and_names_setup() {
        let mut b = QuipuBackend::read_only(
            quipu::store::Store::open_in_memory().unwrap(),
            "urn:quipu:project:test",
        );
        let error = require_profile(&b).unwrap_err();
        assert!(error.message.contains("quipu shapes load seeds"));
        assert!(error.message.contains(PROFILE_REF));
        assert!(b.store().list_shapes().unwrap().is_empty());
        b.store_mut().load_shapes("other", "@prefix sh: <http://www.w3.org/ns/shacl#> . <urn:test:Shape> sh:targetClass <https://schema.org/Action> .", "2026-01-01T00:00:00Z").unwrap();
        assert!(
            require_profile(&b).is_err(),
            "matching vocabulary is not the pinned profile"
        );
        b.store_mut()
            .load_shapes("seeds", crate::vocab::SHAPES_TURTLE, "2026-01-01T00:00:00Z")
            .unwrap();
        require_profile(&b).unwrap();
    }
}
