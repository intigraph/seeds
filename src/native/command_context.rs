//! CLI command attribution without arguments, global environment mutation, or actors.
use std::cell::Cell;

thread_local! {
    static SOURCE: Cell<&'static str> = const { Cell::new("seeds") };
}

pub(super) fn source() -> &'static str {
    SOURCE.with(Cell::get)
}

pub(super) struct Scope(&'static str);

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
        Self(SOURCE.with(|slot| {
            let outer = slot.get();
            slot.set(if outer == "seeds" { source } else { outer });
            outer
        }))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        SOURCE.with(|slot| slot.set(self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

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
