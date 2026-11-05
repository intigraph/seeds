use super::*;
use crate::{
    backend::Backend,
    engine::{self, CreateReq},
    quipu_backend::QuipuBackend,
};
use std::cell::{Cell, RefCell};
const GRAPH: &str = "https://seeds.example/project/fixture";
fn ctx() -> Ctx {
    Ctx {
        now: "2026-01-01T00:00:00Z".into(),
        actor: "fixture".into(),
        prefix: "sd".into(),
        claims: Default::default(),
    }
}
fn item() -> Item {
    Item {
        title: "Fixture".into(),
        description: "Body".into(),
        status: "open".into(),
        priority: 2,
        issue_type: "task".into(),
        assignee: None,
        labels: vec![],
        parent: None,
        blocked_on: vec![],
        comments: vec![],
    }
}
#[derive(Default)]
struct Fixture {
    rows: RefCell<BTreeMap<String, Remote>>,
    creates: Cell<usize>,
    updates: Cell<usize>,
    relations: Cell<usize>,
    fail_after: Cell<Option<usize>>,
}
impl Adapter for Fixture {
    fn find(&self, origin: &str) -> Result<Option<Remote>> {
        Ok(self
            .rows
            .borrow()
            .values()
            .find(|r| r.origin.as_deref() == Some(origin))
            .cloned())
    }
    fn get(&self, id: &str) -> Result<Remote> {
        self.rows
            .borrow()
            .get(id)
            .cloned()
            .ok_or_else(|| SdError::not_found(id))
    }
    fn get_link(&self, link: &str) -> Result<Remote> {
        self.rows
            .borrow()
            .values()
            .find(|r| r.link == link)
            .cloned()
            .ok_or_else(|| SdError::not_found("link"))
    }
    fn put(&self, origin: &str, item: &Item, current: Option<&Remote>) -> Result<Remote> {
        if self.fail_after.get() == Some(self.creates.get()) {
            return Err(SdError::refused("fixture write refused"));
        }
        let id = if let Some(r) = current {
            self.updates.set(self.updates.get() + 1);
            r.id.clone()
        } else {
            self.creates.set(self.creates.get() + 1);
            self.creates.get().to_string()
        };
        let r = Remote {
            id: id.clone(),
            origin: Some(origin.into()),
            link: format!("https://github.example/owner/repo/issues/{id}"),
            item: item.clone(),
        };
        self.rows.borrow_mut().insert(id, r.clone());
        Ok(r)
    }
    fn relate(
        &self,
        r: &Remote,
        item: &Item,
        targets: &BTreeMap<String, Remote>,
        _: &str,
    ) -> Result<()> {
        self.relations.set(self.relations.get() + 1);
        let mut rows = self.rows.borrow_mut();
        let row = rows.get_mut(&r.id).unwrap();
        row.item.parent = item.parent.as_ref().map(|id| targets[id].link.clone());
        row.item.blocked_on = item
            .blocked_on
            .iter()
            .map(|id| targets[id].link.clone())
            .collect();
        Ok(())
    }
}
fn seed(b: &mut QuipuBackend, title: &str) -> String {
    engine::create(
        b,
        &ctx(),
        &CreateReq {
            title: title.into(),
            ..Default::default()
        },
    )
    .unwrap()
    .0
    .id
}
fn add_remote(f: &Fixture, id: &str, i: Item) {
    f.rows.borrow_mut().insert(
        id.into(),
        Remote {
            id: id.into(),
            origin: None,
            link: format!("https://github.example/owner/repo/issues/{id}"),
            item: i,
        },
    );
}
#[test]
fn export_preview_does_not_mutate_either_side() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let id = seed(&mut b, "one");
    let tx = b.snapshot(None).unwrap().tx;
    let f = Fixture::default();
    let plan = records::export(&mut b, &ctx(), GRAPH, &f, &[id], false, false).unwrap();
    assert_eq!(plan[0]["action"], "create");
    assert_eq!(f.creates.get(), 0);
    assert_eq!(b.snapshot(None).unwrap().tx, tx);
}
#[test]
fn export_replay_updates_linked_item_without_duplicate() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let id = seed(&mut b, "one");
    let f = Fixture::default();
    records::export(
        &mut b,
        &ctx(),
        GRAPH,
        &f,
        std::slice::from_ref(&id),
        true,
        false,
    )
    .unwrap();
    records::export(
        &mut b,
        &ctx(),
        GRAPH,
        &f,
        std::slice::from_ref(&id),
        true,
        false,
    )
    .unwrap();
    assert_eq!(f.creates.get(), 1);
    assert_eq!(f.updates.get(), 0);
    engine::update(
        &mut b,
        &ctx(),
        std::slice::from_ref(&id),
        &engine::UpdateReq {
            title: Some("changed".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(records::export(
        &mut b,
        &ctx(),
        GRAPH,
        &f,
        std::slice::from_ref(&id),
        true,
        false
    )
    .is_err());
    assert_eq!(f.updates.get(), 0);
    records::export(&mut b, &ctx(), GRAPH, &f, &[id], true, true).unwrap();
    assert_eq!(f.creates.get(), 1);
    assert_eq!(f.updates.get(), 1);
}
#[test]
fn export_recovers_origin_after_lost_link_receipt() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let id = seed(&mut b, "one");
    let snap = b.snapshot_items(std::slice::from_ref(&id)).unwrap();
    let f = Fixture::default();
    let i = Item {
        title: "one".into(),
        description: String::new(),
        ..item()
    };
    f.put(&format!("{GRAPH}#{id}"), &i, None).unwrap();
    assert!(snap.get(&id).unwrap().external_ref.is_none());
    records::export(
        &mut b,
        &ctx(),
        GRAPH,
        &f,
        std::slice::from_ref(&id),
        true,
        false,
    )
    .unwrap();
    assert_eq!(f.creates.get(), 1);
    assert!(b
        .snapshot_items(std::slice::from_ref(&id))
        .unwrap()
        .get(&id)
        .unwrap()
        .external_ref
        .is_some());
}
#[test]
fn export_refusal_preserves_successful_prior_link() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let first = seed(&mut b, "first");
    let second = seed(&mut b, "second");
    let f = Fixture::default();
    f.fail_after.set(Some(1));
    assert!(records::export(
        &mut b,
        &ctx(),
        GRAPH,
        &f,
        &[first.clone(), second.clone()],
        true,
        false
    )
    .is_err());
    let snap = b.snapshot_items(&[first.clone(), second.clone()]).unwrap();
    assert!(snap.get(&first).unwrap().external_ref.is_some());
    assert!(snap.get(&second).unwrap().external_ref.is_none());
    assert_eq!(f.creates.get(), 1);
}
#[test]
fn import_preview_and_replay_keep_ids_comments_and_transactions() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let f = Fixture::default();
    let mut i = item();
    i.comments.push(Note {
        key: "comment-9".into(),
        author: "external".into(),
        at: "2025-12-31T23:00:00Z".into(),
        text: "hello".into(),
    });
    add_remote(&f, "1", i);
    let p = records::import(&mut b, &ctx(), GRAPH, &f, &["1".into()], false, false).unwrap();
    assert_eq!(p[0]["action"], "create");
    assert!(b.snapshot(None).unwrap().seeds.is_empty());
    records::import(&mut b, &ctx(), GRAPH, &f, &["1".into()], true, false).unwrap();
    let before = b.snapshot(None).unwrap();
    assert_eq!(before.seeds.len(), 1);
    assert_eq!(before.comments.len(), 1);
    records::import(&mut b, &ctx(), GRAPH, &f, &["1".into()], true, false).unwrap();
    let after = b.snapshot(None).unwrap();
    assert_eq!(after.tx, before.tx);
    assert_eq!(after.comments.len(), 1);
}
#[test]
fn import_exports_back_to_original_seed() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let id = seed(&mut b, "one");
    engine::comment_add(&mut b, &ctx(), &id, "Original immutable comment", None).unwrap();
    let f = Fixture::default();
    records::export(
        &mut b,
        &ctx(),
        GRAPH,
        &f,
        std::slice::from_ref(&id),
        true,
        false,
    )
    .unwrap();
    let before = b.snapshot(None).unwrap();
    let p = records::import(&mut b, &ctx(), GRAPH, &f, &["1".into()], true, false).unwrap();
    assert_eq!(p[0]["seed"], id);
    let after = b.snapshot(None).unwrap();
    assert_eq!(after.seeds.len(), 1);
    assert_eq!(after.comments, before.comments);
    assert_eq!(after.tx, before.tx);
    f.rows.borrow_mut().get_mut("1").unwrap().item.comments[0].text = "Edited history".into();
    assert!(records::import(&mut b, &ctx(), GRAPH, &f, &["1".into()], true, true).is_err());
    assert_eq!(b.snapshot(None).unwrap().tx, before.tx);
}
#[test]
fn import_conflict_is_atomic_and_explicit_source_choice_updates() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let f = Fixture::default();
    add_remote(&f, "1", item());
    records::import(&mut b, &ctx(), GRAPH, &f, &["1".into()], true, false).unwrap();
    let tx = b.snapshot(None).unwrap().tx;
    f.rows.borrow_mut().get_mut("1").unwrap().item.title = "changed".into();
    add_remote(&f, "2", item());
    assert!(records::import(
        &mut b,
        &ctx(),
        GRAPH,
        &f,
        &["1".into(), "2".into()],
        true,
        false
    )
    .is_err());
    assert_eq!(b.snapshot(None).unwrap().tx, tx);
    records::import(
        &mut b,
        &ctx(),
        GRAPH,
        &f,
        &["1".into(), "2".into()],
        true,
        true,
    )
    .unwrap();
    assert_eq!(b.snapshot(None).unwrap().seeds.len(), 2);
}
#[test]
fn import_parent_dependencies_require_closure_and_reject_cycle() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let f = Fixture::default();
    add_remote(&f, "1", item());
    let mut child = item();
    child.parent = Some("https://github.example/owner/repo/issues/1".into());
    child.blocked_on = vec![child.parent.clone().unwrap()];
    add_remote(&f, "2", child);
    assert!(records::import(&mut b, &ctx(), GRAPH, &f, &["2".into()], true, false).is_err());
    assert!(b.snapshot(None).unwrap().seeds.is_empty());
    records::import(
        &mut b,
        &ctx(),
        GRAPH,
        &f,
        &["1".into(), "2".into()],
        true,
        false,
    )
    .unwrap();
    let tx = b.snapshot(None).unwrap().tx;
    f.rows.borrow_mut().get_mut("1").unwrap().item.blocked_on =
        vec!["https://github.example/owner/repo/issues/2".into()];
    assert!(records::import(
        &mut b,
        &ctx(),
        GRAPH,
        &f,
        &["1".into(), "2".into()],
        true,
        true
    )
    .is_err());
    assert_eq!(b.snapshot(None).unwrap().tx, tx);
}
#[test]
fn imported_comment_edits_are_not_silently_discarded() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let f = Fixture::default();
    let mut i = item();
    i.comments = vec![Note {
        key: "1".into(),
        author: "a".into(),
        at: "2025-12-01".into(),
        text: "one".into(),
    }];
    add_remote(&f, "1", i);
    records::import(&mut b, &ctx(), GRAPH, &f, &["1".into()], true, false).unwrap();
    let tx = b.snapshot(None).unwrap().tx;
    f.rows.borrow_mut().get_mut("1").unwrap().item.comments[0].text = "edited".into();
    assert!(records::import(&mut b, &ctx(), GRAPH, &f, &["1".into()], true, true).is_err());
    assert_eq!(b.snapshot(None).unwrap().tx, tx);
}
#[test]
fn malformed_unicode_marker_cannot_panic() {
    assert!(decode_origin("<!-- seeds-origin:🌲 -->", "<!-- seeds-origin:", " -->").is_none());
}

#[test]
fn mode_one_preview_reads_changed_pendant_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let store = root.path().join("working.db");
    let dir = root.path().join("pendant");
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let id = seed(&mut b, "before");
    super::super::store::export_to_pendant(&b, &store, &dir).unwrap();
    let marker = std::fs::read(super::super::store::marker_path(&store)).unwrap();
    let mut changed = QuipuBackend::in_memory(GRAPH).unwrap();
    let original = crate::pendant::read(&crate::pendant::export(&b).unwrap()).unwrap();
    crate::sync::import(&mut changed, &ctx(), &original.snapshot, None, true).unwrap();
    engine::update(
        &mut changed,
        &ctx(),
        std::slice::from_ref(&id),
        &engine::UpdateReq {
            title: Some("after".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let disk = crate::pendant::export(&changed).unwrap();
    super::super::store::write_pendant_dir(&dir, &disk).unwrap();
    let preview = super::super::store::preview_hydrated(&b, &store, &dir, &ctx(), GRAPH).unwrap();
    assert_eq!(
        preview
            .snapshot_items(std::slice::from_ref(&id))
            .unwrap()
            .get(&id)
            .unwrap()
            .title,
        "after"
    );
    assert_eq!(
        b.snapshot_items(std::slice::from_ref(&id))
            .unwrap()
            .get(&id)
            .unwrap()
            .title,
        "before"
    );
    assert_eq!(
        std::fs::read(super::super::store::marker_path(&store)).unwrap(),
        marker
    );
    assert_eq!(
        super::super::store::read_pendant_dir(&dir)
            .unwrap()
            .unwrap()
            .files,
        disk.files
    );
}

#[test]
fn reserved_description_markers_refuse_before_external_writes() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let id = seed(&mut b, "one");
    engine::update(
        &mut b,
        &ctx(),
        std::slice::from_ref(&id),
        &engine::UpdateReq {
            description: Some("Body\n\n## Seeds dependencies\nOriginal user content".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let f = Fixture::default();
    assert!(records::export(&mut b, &ctx(), GRAPH, &f, &[id], true, false).is_err());
    assert_eq!(f.creates.get(), 0);
}

#[test]
fn repository_dot_segments_refuse_before_credential_read_or_http() {
    for repository in ["../other", "owner/..", "./repo", "owner/."] {
        let args = Args {
            transfer_principal: Some("fixture".into()),
            transfer_credential_fd: Some("2147483647".into()),
            ..Default::default()
        };
        let client = http::Client::new("http://127.0.0.1:1", &args).unwrap();
        let result = github::Github::new(
            Config {
                endpoint: "http://127.0.0.1:1".into(),
                repository: Some(repository.into()),
            },
            client,
        );
        assert!(result.is_err());
    }
}
