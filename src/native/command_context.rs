//! CLI command attribution without arguments, global environment mutation, or actors.
use std::cell::Cell;

thread_local! {
    static CLIENT_COMMAND: Cell<Option<&'static str>> = const { Cell::new(None) };
    static SOURCE: Cell<&'static str> = const { Cell::new("seeds") };
}

pub(super) fn source() -> &'static str {
    SOURCE.with(Cell::get)
}

/// The metric label uses only the parsed top-level command. Library calls
/// outside a CLI scope retain the historical producer label.
pub(super) fn client() -> String {
    CLIENT_COMMAND.with(|slot| match slot.get() {
        Some(command) => format!("seeds/{command}"),
        None => "seeds".into(),
    })
}

pub(super) struct Scope {
    source: &'static str,
    client: Option<&'static str>,
}

impl Scope {
    pub(super) fn new(command: &super::cli::Command) -> Self {
        use super::cli::Command;
        let source = match command {
            Command::Show(_) => "sd-show",
            Command::List(_) => "sd-list",
            Command::Ready(_) => "sd-ready",
            Command::Search(_) => "sd-search",
            Command::Stats(_) => "sd-stats",
            Command::Count(_) => "sd-count",
            Command::Create(_) => "sd-create",
            Command::Update(_) => "sd-update",
            Command::Close(_) => "sd-close",
            Command::Comments { .. } => "sd-comments",
            Command::Where => "sd-where",
            Command::Info(_) => "sd-info",
            _ => "sd-other",
        };
        let source = SOURCE.with(|slot| {
            let outer = slot.get();
            slot.set(if outer == "seeds" { source } else { outer });
            outer
        });
        // Command::name is an exhaustive enum match of static names. No
        // argument, subject, actor, raw argv or environment enters this label.
        let command = command
            .name()
            .split_ascii_whitespace()
            .next()
            .expect("command name");
        let client = CLIENT_COMMAND.with(|slot| {
            let outer = slot.get();
            slot.set(outer.or(Some(command)));
            outer
        });
        Self { source, client }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        SOURCE.with(|slot| slot.set(self.source));
        CLIENT_COMMAND.with(|slot| slot.set(self.client));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn metric_client_is_command_only_for_every_request_kind() {
        let cli =
            super::super::cli::Cli::try_parse_from(["sd", "show", "private-subject"]).unwrap();
        assert_eq!(client(), "seeds");
        {
            let _scope = Scope::new(&cli.command);
            assert_eq!(client(), "seeds/show");
            let requests = [
                super::super::provenance::apply(ureq::get("http://localhost/unused")),
                super::super::provenance::apply_read(ureq::get("http://localhost/unused"), "query"),
                super::super::provenance::apply_signed(ureq::get("http://localhost/unused")),
            ];
            for request in requests {
                assert_eq!(request.header("X-Quipu-Client"), Some("seeds/show"));
            }
        }
        assert_eq!(client(), "seeds");
    }

    #[test]
    fn nested_command_labels_and_parallel_scopes_do_not_leak() {
        let cli = super::super::cli::Cli::try_parse_from(["sd", "list"]).unwrap();
        let _outer = Scope::new(&cli.command);
        let inner = super::super::cli::Cli::try_parse_from(["sd", "show", "private-id"]).unwrap();
        {
            let _inner = Scope::new(&inner.command);
            assert_eq!(client(), "seeds/list");
        }
        let other = std::thread::spawn(|| {
            assert_eq!(client(), "seeds");
            let cli = super::super::cli::Cli::try_parse_from(["sd", "ready"]).unwrap();
            let _scope = Scope::new(&cli.command);
            assert_eq!(client(), "seeds/ready");
        });
        other.join().unwrap();
        assert_eq!(client(), "seeds/list");
    }

    #[test]
    fn nested_verbs_share_one_bounded_top_level_label() {
        for argv in [
            vec!["sd", "comments", "add", "private-id", "private-body"],
            vec!["sd", "comments", "list", "other-private-id"],
        ] {
            let cli = super::super::cli::Cli::try_parse_from(argv).unwrap();
            let _scope = Scope::new(&cli.command);
            assert_eq!(client(), "seeds/comments");
        }
    }

    #[test]
    fn demo_command_attribution_before_and_after() {
        let before = super::super::provenance::read_headers_from(|_| None, || None, "query");
        let cli =
            super::super::cli::Cli::try_parse_from(["sd", "show", "private-subject"]).unwrap();
        let _scope = Scope::new(&cli.command);
        let after = super::super::provenance::read_headers_from(|_| None, || None, "query");
        let source = |headers: &[(&str, String)]| {
            headers
                .iter()
                .find(|(key, _)| *key == "X-Quipu-Source")
                .unwrap()
                .1
                .clone()
        };
        assert_eq!(source(&before), "seeds");
        assert_eq!(source(&after), "sd-show");
        println!(
            "before: {}",
            serde_json::json!({"source": source(&before), "operation": "query"})
        );
        println!(
            "after: {}",
            serde_json::json!({"source": source(&after), "operation": "query"})
        );
        assert!(!format!("{after:?}").contains("private-subject"));
    }

    #[test]
    fn parsed_command_not_arguments_is_retained_and_restored() {
        let cli =
            super::super::cli::Cli::try_parse_from(["sd", "show", "private-subject"]).unwrap();
        let before = source();
        {
            let _scope = Scope::new(&cli.command);
            assert_eq!(source(), "sd-show");
            let headers = super::super::provenance::read_headers_from(|_| None, || None, "query");
            assert!(headers
                .iter()
                .any(|(k, v)| *k == "X-Quipu-Source" && v == "sd-show"));
            assert!(!format!("{headers:?}").contains("private-subject"));
            let inherited = super::super::provenance::read_headers_from(
                |key| (key == "QUIPU_SOURCE").then(|| "st-anchor".into()),
                || None,
                "query",
            );
            assert!(inherited
                .iter()
                .any(|(k, v)| *k == "X-Quipu-Source" && v == "st-anchor"));
        }
        assert_eq!(source(), before);
    }

    #[test]
    fn nested_scope_restores_after_failure_without_overwriting_actor() {
        let outer = super::super::cli::Cli::try_parse_from(["sd", "list"]).unwrap();
        let inner = super::super::cli::Cli::try_parse_from(["sd", "where"]).unwrap();
        let _scope = Scope::new(&outer.command);
        let _ = std::panic::catch_unwind(|| {
            let _nested = Scope::new(&inner.command);
            assert_eq!(source(), "sd-list");
            panic!("control");
        });
        assert_eq!(source(), "sd-list");
        let headers = super::super::provenance::headers_from(
            |key| (key == "QUIPU_AGENT").then(|| "writer-control".into()),
            || None,
        );
        assert!(headers
            .iter()
            .any(|(k, v)| *k == "X-Quipu-Agent" && v == "writer-control"));
    }
}
