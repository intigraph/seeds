//! Bounded labels and journal volume, independent of refusal traffic/contents.
use super::refusal_reason;

const LOG_EVERY_MS: u64 = 60_000;
// Stable client reason codes and journal counter labels. Do not derive labels
// from free text; new producer messages use read_refused until classified.
const CODES: [&str; 7] = [
    "freshness_unknown",
    "proof_budget",
    "binding_mismatch",
    "transport_unavailable",
    "protocol_invalid",
    "replica_unavailable",
    "read_refused",
];

// Existing error producers return text. Keep classification here, before the
// bounded wire/journal rendering, rather than using variable text as labels.
fn classify(message: &str) -> usize {
    if message.contains("replica bounded freshness is UNKNOWN") {
        0
    } else if message.contains("proof budget") {
        1
    } else if message.contains("binding") || message.contains("source or epoch changed") {
        2
    } else if message.contains("local transport") {
        3
    } else if message.contains("protocol") || message.contains("IPC frame") {
        4
    } else if message.contains("replica") {
        5
    } else {
        6
    }
}

#[derive(Default)]
pub(super) struct Refusals {
    totals: [u64; CODES.len()],
    suppressed: [u64; CODES.len()],
    last_log: [Option<u64>; CODES.len()],
}

impl Refusals {
    fn observe(&mut self, now: u64, message: &str) -> (String, Option<String>) {
        let i = classify(message);
        self.totals[i] = self.totals[i].saturating_add(1);
        // Prefix first: truncating a long underlying reason cannot erase code.
        // Retain the existing Result<Reply,String> IPC envelope for old clients.
        let wire = refusal_reason(&format!(
            "push read UNKNOWN [reason={}]: {message}",
            CODES[i]
        ));
        let log = if self.last_log[i].is_none_or(|last| now.saturating_sub(last) >= LOG_EVERY_MS) {
            self.last_log[i] = Some(now);
            let suppressed = std::mem::take(&mut self.suppressed[i]);
            Some(format!(
                "sd read-stream: refusal reason={} total={} suppressed={} detail={}",
                CODES[i], self.totals[i], suppressed, wire
            ))
        } else {
            self.suppressed[i] = self.suppressed[i].saturating_add(1);
            None
        };
        (wire, log)
    }

    pub(super) fn report(&mut self, now: u64, message: &str) -> String {
        let (wire, log) = self.observe(now, message);
        if let Some(log) = log {
            eprintln!("{log}");
        }
        wire
    }

    // All fixed labels, including zeros; every refused read is counted even
    // when its individual journal entry is rate-limited or its client left.
    pub(super) fn counters(&self) -> String {
        CODES
            .iter()
            .enumerate()
            .map(|(i, code)| {
                format!(
                    "refusals_total{{reason={code}}}={} refusals_suppressed_pending{{reason={code}}}={}",
                    self.totals[i], self.suppressed[i]
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_finite_and_distinguish_refusal_classes() {
        for (message, code) in [
            ("replica bounded freshness is UNKNOWN", "freshness_unknown"),
            ("read exceeded proof budget (bound 9000 ms)", "proof_budget"),
            (
                "read request differs from admitted binding",
                "binding_mismatch",
            ),
            (
                "local transport or pinned input unavailable",
                "transport_unavailable",
            ),
            ("invalid bounded protocol input", "protocol_invalid"),
            ("replica bootstrap is incomplete", "replica_unavailable"),
            ("unclassified engine error", "read_refused"),
        ] {
            assert_eq!(CODES[classify(message)], code);
        }
    }

    #[test]
    fn flood_is_counted_but_journal_is_bounded_and_other_reasons_still_log() {
        let mut state = Refusals::default();
        let message = "replica bounded freshness is UNKNOWN after waiting 5006 ms (bound 8695 ms, last frame 7741 ms)";
        let (wire, first) = state.observe(0, message);
        assert!(wire.contains("[reason=freshness_unknown]"));
        assert!(wire.contains("bound 8695 ms"));
        assert!(first.unwrap().contains("last frame 7741 ms"));
        for now in 1..60_000 {
            assert!(state.observe(now, message).1.is_none());
        }
        assert!(state
            .observe(59_999, "read exceeded proof budget")
            .1
            .is_some());
        let (_, next) = state.observe(60_000, message);
        let next = next.unwrap();
        assert!(next.contains("total=60001 suppressed=59999"));
        assert!(state
            .counters()
            .contains("refusals_total{reason=freshness_unknown}=60001"));
        assert!(state
            .counters()
            .contains("refusals_total{reason=proof_budget}=1"));
    }

    #[test]
    fn unknown_text_never_becomes_a_label_and_wire_is_bounded_single_line() {
        let mut state = Refusals::default();
        for text in ["one arbitrary error", "another arbitrary error"] {
            let (wire, _) = state.observe(1, text);
            assert!(wire.contains("[reason=read_refused]"));
            assert!(super::super::relayed_refusal(&wire)
                .message
                .contains("[reason=read_refused]"));
        }
        let (wire, log) = state.observe(60_001, &format!("{}\n\r\t\u{1b}", "x".repeat(1000)));
        assert!(wire.chars().count() <= super::super::MAX_REASON + 1);
        assert!(!wire.chars().any(char::is_control));
        assert!(!log.unwrap().chars().any(char::is_control));
        assert!(state
            .counters()
            .contains("refusals_total{reason=read_refused}=3"));
    }
}
