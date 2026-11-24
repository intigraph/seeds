//! Native create admission, including the no-write refusal and atomic override.
#![cfg(feature = "native")]
// Native integration tests intentionally spawn sd and write isolated fixtures.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use serde_json::Value;
use std::process::{Command, Output};

struct Sandbox(tempfile::TempDir);
impl Sandbox {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_sd"));
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.0.path())
            .env("XDG_CONFIG_HOME", self.0.path())
            .env("SEEDS_ACTOR", "fixture-author")
            .env("SEEDS_PREFIX", "fixture")
            .arg("--store")
            .arg(self.0.path().join("board.db"))
            .args(args)
            .current_dir(self.0.path());
        cmd.output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let result = self.run(args);
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    }
    fn count(&self) -> Value {
        serde_json::from_str(&self.ok(&["count", "--json"])).unwrap()
    }
}

#[test]
fn same_title_refuses_with_existing_id_and_preserves_board() {
    let s = Sandbox::new();
    let id = s.ok(&["create", "Repair parser authentication", "--silent"]);
    let before = s.count();
    for title in [
        "Repair parser authentication",
        "Repair parser authenticaton",
    ] {
        let refused = s.run(&["create", title, "--json"]);
        assert_eq!(refused.status.code(), Some(5));
        let error = String::from_utf8_lossy(&refused.stderr);
        assert!(error.contains(id.trim()), "{error}");
        assert!(error.contains("comment on"), "{error}");
        assert_eq!(s.count(), before);
    }
}

#[test]
fn override_requires_candidate_and_reason_and_commits_audit_with_related_edge() {
    let s = Sandbox::new();
    let id = s.ok(&["create", "Repair parser authentication", "--silent"]);
    for args in [
        vec![
            "create",
            "Repair parser authentication",
            "--new-despite",
            id.trim(),
        ],
        vec![
            "create",
            "Repair parser authentication",
            "--new-despite",
            id.trim(),
            "--because",
            " ",
        ],
        vec![
            "create",
            "Repair parser authentication",
            "--new-despite",
            "fixture-missing",
            "--because",
            "separate deployment",
        ],
    ] {
        assert!(!s.run(&args).status.success());
    }
    let before_preview = s.count();
    let preview: Value = serde_json::from_str(&s.ok(&[
        "create",
        "Repair parser authentication",
        "--new-despite",
        id.trim(),
        "--because",
        "separate deployment",
        "--dry-run",
        "--json",
    ]))
    .unwrap();
    assert_eq!(preview["dependency_count"], 1);
    assert_eq!(s.count(), before_preview);
    let new_id = s.ok(&[
        "create",
        "Repair parser authentication",
        "--new-despite",
        id.trim(),
        "--because",
        "separate deployment",
        "--silent",
    ]);
    let shown: Value = serde_json::from_str(&s.ok(&["show", new_id.trim(), "--json"])).unwrap();
    let seed = &shown[0];
    assert_eq!(seed["created_by"], "fixture-author");
    let comments = seed["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["author"], "fixture-author");
    let audit: Value = serde_json::from_str(comments[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(audit["new_despite"][0], id.trim());
    assert_eq!(audit["because"], "separate deployment");
    let deps = s.ok(&["dep", "list", new_id.trim(), "--json"]);
    assert!(
        deps.contains(id.trim()) && deps.contains("related"),
        "{deps}"
    );
}

#[test]
fn closed_items_do_not_refuse_and_distinct_work_is_admitted() {
    let s = Sandbox::new();
    let id = s.ok(&["create", "Repair parser authentication", "--silent"]);
    s.ok(&["close", id.trim(), "--reason", "fixed"]);
    s.ok(&["create", "Repair parser authentication", "--silent"]);
    s.ok(&["create", "Paint the storage cabinet", "--silent"]);
}

#[test]
fn quick_bulk_and_dry_run_cannot_bypass_refusal() {
    let s = Sandbox::new();
    s.ok(&["create", "Repair parser authentication", "--silent"]);
    for args in [
        vec!["q", "Repair parser authentication"],
        vec!["create", "Repair parser authentication", "--dry-run"],
    ] {
        assert_eq!(s.run(&args).status.code(), Some(5));
    }
    std::fs::write(
        s.0.path().join("items.md"),
        "## Repair parser authentication\n\n## Separate issue\n",
    )
    .unwrap();
    let before = s.count();
    assert_eq!(
        s.run(&["create", "--file", "items.md"]).status.code(),
        Some(5)
    );
    assert_eq!(s.count(), before);
}

#[test]
fn workflow_retry_returns_existing_seed() {
    let s = Sandbox::new();
    let args = [
        "create",
        "Repair parser authentication",
        "--workflow-run",
        "fixture-run",
        "--step",
        "repair",
        "--silent",
    ];
    assert_eq!(s.ok(&args), s.ok(&args));
}

#[test]
fn shared_machine_template_does_not_treat_different_episode_ids_as_a_typo() {
    let s = Sandbox::new();
    s.ok(&["create", "Escalation episode 0123456789ab", "--silent"]);
    s.ok(&["create", "Escalation episode abcdef012345", "--silent"]);
    assert_eq!(
        s.run(&["create", "Escalation episode 0123456789ab"])
            .status
            .code(),
        Some(5)
    );
}
