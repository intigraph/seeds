use seeds::backend::{Backend, Ctx, WriteBatch};
use seeds::engine::{self, CreateReq};
use seeds::error::{Result, SdError};
use seeds::model::Snapshot;
use seeds::quipu_backend::QuipuBackend;
use std::cell::{Cell, RefCell};

struct ClosedReads {
    snap: Snapshot,
    since: RefCell<Vec<Option<String>>>,
    pins: RefCell<Vec<Option<u64>>>,
    fail: Cell<bool>,
}

impl Backend for ClosedReads {
    fn scoped_reads(&self) -> bool {
        true
    }
    fn snapshot(&self, at: Option<u64>) -> Result<Snapshot> {
        assert!(at.is_some(), "current changelog hydrated the whole board");
        self.pins.borrow_mut().push(at);
        Ok(self.snap.clone())
    }
    fn snapshot_closed_since(&self, since: Option<&str>) -> Result<Snapshot> {
        self.since.borrow_mut().push(since.map(str::to_string));
        if self.fail.get() {
            return Err(SdError::failed("closed cohort unavailable"));
        }
        // Deliberately return a superset: exact filtering remains in core.
        Ok(self.snap.clone())
    }
    fn ready_ids(&self, _: Option<u64>) -> Result<Vec<String>> {
        unreachable!("changelog does not need ready computation")
    }
    fn claims_of(&self, _: &str) -> Result<Vec<(u64, seeds::backend::Claims)>> {
        unreachable!("changelog does not read write attribution")
    }
    fn commit(&mut self, _: &WriteBatch, _: &Ctx) -> Result<u64> {
        unreachable!("changelog is read-only")
    }
}

fn ctx(now: &str) -> Ctx {
    Ctx {
        now: now.into(),
        actor: "fixture".into(),
        prefix: "scope".into(),
        claims: Default::default(),
    }
}

#[test]
fn current_changelog_scopes_and_rechecks_supersets_but_history_stays_pinned() {
    let mut b = QuipuBackend::in_memory("https://example.test/board").unwrap();
    let mut ids = Vec::new();
    for title in ["old closed", "recent closed", "unrelated open"] {
        ids.push(
            engine::create(
                &mut b,
                &ctx("2026-09-01T00:00:00Z"),
                &CreateReq {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .0
            .id,
        );
    }
    engine::close(&mut b, &ctx("2026-09-02T00:00:00Z"), &ids[..1], None, true).unwrap();
    engine::close(&mut b, &ctx("2026-09-20T00:00:00Z"), &ids[1..2], None, true).unwrap();
    let since = "2026-09-14T00:00:00Z";
    let expected = engine::changelog(&b, Some(since), None).unwrap();
    let reader = ClosedReads {
        snap: b.snapshot(None).unwrap(),
        since: RefCell::new(Vec::new()),
        pins: RefCell::new(Vec::new()),
        fail: Cell::new(false),
    };
    assert_eq!(
        engine::changelog(&reader, Some(since), None).unwrap(),
        expected
    );
    assert_eq!(expected[0].issues[0].id, ids[1]);
    assert_eq!(expected[0].issues.len(), 1);
    assert_eq!(*reader.since.borrow(), vec![Some(since.to_string())]);
    assert_eq!(
        engine::changelog(&reader, None, None).unwrap(),
        engine::changelog(&b, None, None).unwrap()
    );
    let pin = reader.snap.tx;
    assert_eq!(
        engine::changelog(&reader, Some(since), Some(pin)).unwrap(),
        expected
    );
    assert_eq!(*reader.pins.borrow(), vec![Some(pin)]);
    assert_eq!(
        reader.since.borrow().len(),
        2,
        "history used a current cohort"
    );
    reader.fail.set(true);
    assert!(engine::changelog(&reader, Some(since), None).is_err());
    assert_eq!(
        reader.pins.borrow().len(),
        1,
        "failure fell back to a full scan"
    );
}
