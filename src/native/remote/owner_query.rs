//! Owner-list query planning without changing decoded default semantics.
use super::*;

impl RemoteBackend {
    pub(super) fn owner_seed_query_pattern(
        &self,
        _graph: &str,
        query: &crate::backend::SeedQuery,
    ) -> Result<Option<String>> {
        // The remote graph is shared and mutable. Even a fresh negative ASK
        // can become stale before discovery, and this protocol supplies no
        // immutable revision binding the two reads. Keep default-open matching
        // in the discovery query itself so external additions/status removals
        // cannot disappear. Status-first planning of regular leaves is retained.
        Ok(vocab::seed_query_pattern_var(query, "?s"))
    }
}
