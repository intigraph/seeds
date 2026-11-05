# GitHub Issues transfers

`sd export --to github` and `sd import --from github` perform one-shot,
explicitly selected transfers. They preview by default. They do not schedule
synchronization. The provider-neutral adapter boundary leaves room for other
task managers; GitHub Issues is the implemented adapter.

Create a credential-free adapter TOML:

```toml
endpoint = "https://api.github.com"
repository = "example/work"
```

Use the named-credential launcher to supply a private anonymous inherited
file descriptor containing the GitHub credential. `--transfer-principal`
and `--transfer-credential-fd` select that credential independently of Quipu
credentials. Never pass the credential itself as an argument or put it in
the TOML. The existing descriptor checks require private ownership and
permissions. Requests require HTTPS, except loopback fixture servers; they
never follow redirects or automatically retry writes.

```bash
# Within the credential launcher's process, with descriptor 3 inherited:
sd export --to github --adapter-config github.toml --ids sd-one,sd-two \
  --transfer-principal github-work --transfer-credential-fd 3
# Review the returned operations, then repeat with --execute.
sd import --from github --adapter-config github.toml --ids 12,13 \
  --transfer-principal github-work --transfer-credential-fd 3
```

Select 1–100 exact IDs: seed IDs for export, issue numbers for import.
Include every parent and dependency target in the same selection. Missing
closure and cycles refuse import rather than creating dangling edges.
An existing linked target with different fields is a conflict. Reconcile it,
or explicitly choose the source with `--overwrite --execute` after preview.
No automatic merge chooses a side.

| Seed field | GitHub representation |
|---|---|
| title, description | title and issue body |
| open/in_progress/blocked/closed | native open/closed plus `seeds:status:` label |
| priority 0–4 | `seeds:priority:` label |
| type | `seeds:type:` label |
| assignee | single native assignee; multiple assignees require reconciliation |
| labels | native labels; the `seeds:` prefix is reserved |
| parent | native sub-issue relationship |
| blocked_on | native blocked-by relationships, or visible body section when API returns 404/410 |
| comments | native comments with original attribution and immutable source identity |

Authentication and permission refusals do not trigger the dependency fallback.
Descriptions containing reserved origin markers or the dependency heading are
refused instead of being truncated. The body section is headed `## Seeds dependencies` and lists issue links.
Native closed state wins over an open status label; an open issue carrying a
closed label requires explicit reconciliation. Required mapping labels and
written fields are read back, so silently dropped data cannot count as success.
Pull requests and ephemeral seeds are refused.

Export stores the canonical issue URL as the seed's `external_ref`, through
the normal revision-checked write path. A hidden body marker identifies the
source project and seed. If creation lands but its reply or local link write
fails, a later transfer searches for that marker before creating another
issue. Multiple matches are a conflict. Import derives stable IDs from issue
URLs and reuses original seed IDs for a same-project round trip. Imported
fields and comments commit atomically; unrelated seed fields are preserved.
Existing comments are immutable: edited linked comments require reconciliation.

Exports span two systems and cannot be globally atomic. A partial failure can
leave earlier issue writes or links committed; preserve the result, inspect
both systems, then preview again. HTTP transport failures after writes remain
indeterminate. Pagination is bounded at 100 pages of 100 rows, and hitting
the bound refuses any completeness claim.

`--json` returns `{schema, adapter, direction, dry_run, operations}` with schema
`seeds.transfer/1`; each operation includes `seed`, `action`, `external_ref`,
`conflict` and the mapped `fields`. Pendant import/export retain their existing
formats and behavior when no task-manager adapter is selected.

The loopback-only `tests/task_transfer.py` fixture exercises the real CLI and
HTTP adapter with an inert descriptor credential. It runs in `just check`.
Live acceptance uses a dedicated fixture repository and named credential;
fixture success is separate from live target authorization and acceptance.
