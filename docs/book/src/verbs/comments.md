# sd comments

```bash
sd comments add sd-a3f "found the off-by-one in the lexer"
sd comments add sd-a3f --file notes.md      # safest for long or quoted text
sd comments list sd-a3f --json
```

`comments add` takes the text one way: positional words (joined with spaces),
`-m/--message` (alias `--content`), or `-f/--file`. The author is `--author`,
else the actor. Comments are append-only and numbered 1, 2, 3 per seed; the
number is the comment's `id`.

Current remote `comments list` reads only the named seed and its comments,
including its ephemeral graph. Historical `--at` reads retain transaction
snapshot semantics. A missing item or an unavailable read remains an error.

**`--json`**: one comment (add) or an array (list), each `{id, issue_id,
author, text, created_at}`; `add` also carries `tx`.

Use the plural subcommand: `sd comments add ID -m "Text"`. The legacy guess
`sd comment ID -m "Text"` prints the corrected command. Empty text is refused
with the message/file forms, and adds no comment.
