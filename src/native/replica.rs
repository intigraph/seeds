//! Staleness of a Quipu read replica, as its responses state it.
//!
//! A replica serving a follower database (quipu `server.replica`) refuses
//! writes and stamps every response with `X-Quipu-Replica-*` headers: the last
//! source transaction it holds, when that transaction committed at the
//! source, how long ago the follower last confirmed it was current, and its
//! lag behind the primary. sd passes them on so a reader during an outage can
//! tell it is looking at a copy, and how old the copy is (aegis-0aua).
//!
//! The values are the replica's own claims, read once at connect; "unknown"
//! stays unknown and is never turned into zero.
use serde_json::{json, Value as Json};

const LAG_TX: &str = "x-quipu-replica-lag-tx";
const SOURCE_TX: &str = "x-quipu-replica-source-tx";
const SOURCE_TIME: &str = "x-quipu-replica-source-time";
const CONFIRMED_AGE_S: &str = "x-quipu-replica-confirmed-age-s";

/// What a replica's response said about its staleness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staleness {
    /// The last source transaction the follower holds.
    pub source_tx: Option<u64>,
    /// When that transaction committed at the source.
    pub source_time: Option<String>,
    /// Seconds since the follower last confirmed it was current.
    pub confirmed_age_s: Option<u64>,
    /// Transactions behind the primary.
    pub lag_tx: Option<u64>,
}

impl Staleness {
    /// The staleness a response states, or `None` when it carries none (the
    /// server is not a replica). `header` looks a header up by its name.
    pub fn from_headers<'a>(header: impl Fn(&str) -> Option<&'a str>) -> Option<Self> {
        if header(LAG_TX).is_none() && header(SOURCE_TX).is_none() {
            return None;
        }
        let number = |name| header(name).and_then(|v| v.trim().parse().ok());
        let source_time = header(SOURCE_TIME)
            .map(str::trim)
            .filter(|v| !v.is_empty() && *v != "unknown")
            .map(|v| v.chars().filter(|c| !c.is_control()).take(64).collect());
        Some(Self {
            source_tx: number(SOURCE_TX),
            source_time,
            confirmed_age_s: number(CONFIRMED_AGE_S),
            lag_tx: number(LAG_TX),
        })
    }

    /// The one-line notice for text output.
    pub fn notice(&self) -> String {
        fn or_unknown<T: std::fmt::Display>(v: &Option<T>) -> String {
            v.as_ref()
                .map_or_else(|| "unknown".to_string(), ToString::to_string)
        }
        let committed = self
            .source_time
            .as_ref()
            .map(|t| format!(" (committed {t})"))
            .unwrap_or_default();
        let confirmed = self
            .confirmed_age_s
            .map_or_else(|| "unknown".to_string(), |s| format!("{s} s ago"));
        format!(
            "read from a Quipu read replica; writes are refused. Source tx {}{committed}, \
             confirmed current {confirmed}, lag {} tx.",
            or_unknown(&self.source_tx),
            or_unknown(&self.lag_tx),
        )
    }

    /// The same fields for `--json`, as `{"replica": {...}}`.
    pub fn to_json(&self) -> Json {
        json!({"replica": {
            "writes": "refused",
            "source_tx": self.source_tx,
            "source_time": self.source_time,
            "confirmed_age_s": self.confirmed_age_s,
            "lag_tx": self.lag_tx,
        }})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(pairs: &[(&'static str, &'static str)]) -> Option<Staleness> {
        Staleness::from_headers(|name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v))
    }

    #[test]
    fn a_server_without_replica_headers_is_not_a_replica() {
        assert_eq!(read(&[("x-quipu-client", "seeds")]), None);
    }

    #[test]
    fn a_disconnected_replica_keeps_unknown_as_unknown() {
        let s = read(&[
            (LAG_TX, "unknown"),
            (SOURCE_TX, "6"),
            (SOURCE_TIME, "2026-10-25T09:48:39Z"),
            (CONFIRMED_AGE_S, "21"),
        ])
        .unwrap();
        assert_eq!(s.source_tx, Some(6));
        assert_eq!(s.lag_tx, None);
        assert_eq!(
            s.notice(),
            "read from a Quipu read replica; writes are refused. Source tx 6 (committed \
             2026-10-25T09:48:39Z), confirmed current 21 s ago, lag unknown tx."
        );
        assert_eq!(
            s.to_json(),
            json!({"replica": {"writes": "refused", "source_tx": 6,
                   "source_time": "2026-10-25T09:48:39Z", "confirmed_age_s": 21,
                   "lag_tx": null}})
        );
    }

    #[test]
    fn a_replica_that_never_confirmed_says_so() {
        let s = read(&[(LAG_TX, "unknown"), (CONFIRMED_AGE_S, "unknown")]).unwrap();
        assert_eq!(
            s.notice(),
            "read from a Quipu read replica; writes are refused. Source tx unknown, \
             confirmed current unknown, lag unknown tx."
        );
    }

    #[test]
    fn server_text_cannot_break_the_notice_line() {
        let s = read(&[(SOURCE_TX, "1"), (SOURCE_TIME, "2026\u{1b}[2J\nforged")]).unwrap();
        assert_eq!(s.source_time.as_deref(), Some("2026[2Jforged"));
    }
}
