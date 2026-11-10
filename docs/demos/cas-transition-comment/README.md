# Same-owner CAS receipt binding

The same-owner assignment has revision 2. A new non-empty transition comment
with `if_revision=2` represents a fresh binding, even though status and owner
already match. Base `4cf5b2f5ddb6d8e8473dd9f19c5ad2ec48a11764` returns revision
2 and discards that comment; the candidate returns revision 3 and commits the
comment atomically. The same probe then verifies the comment is absent before
the transaction and that replay with the old expected revision conflicts
without changing the resulting seed or comments.

[Actual before/after transcript](output.txt) comes from the bounded guarded
library run of the same test, with compilation paths omitted. It is a transcript,
not a terminal recording: the bounded runner captured process output without
terminal replay timing. No live tracker was mutated.

```sh
cargo test --no-default-features --test core \
  cas_transition_comment_rebinds_an_unchanged_assignment_atomically \
  -- --exact --nocapture
```

The focused regression went from `0 passed; 1 failed` to `1 passed; 0 failed`.
Ordinary no-op updates, comments without an exact revision condition, and blank
comments retain their existing behavior. This small transcript is retained with
the source demo; full guarded logs are private operational evidence.
