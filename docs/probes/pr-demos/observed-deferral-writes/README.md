# Observed deferral writes

The candidate CLI runs against one disposable, local store. The same item starts
with no observation metadata, is deferred twice, then undeferred. Watch the
exported RDF count change from unknown to 1 to 2, and remain 2 after undefer;
the reproducer also verifies that the original observation date is retained.

This is a candidate lifecycle demonstration, not a historical total, a deployed
board measurement, or a base-binary comparison. Base source:
`1d16b4c999ddcf37e511c35f2c5453bd09900061`. The PR identifies the qualified
implementation revision. The recording contains only synthetic fixture output.
The origin timestamp is checked but omitted from the transcript to keep it stable.

[Recording output](output.txt) and [timing](timing.txt) are retained here with the
[reproducer](reproduce.py); download both to replay with:

```sh
scriptreplay --log-out output.txt --log-timing timing.txt
```

To repeat with your candidate binary:

```sh
python3 reproduce.py /absolute/path/to/sd
```

The new facts are deliberately absent from the stable CLI JSON keys. The demo
reads native pendant RDF instead. Missing metadata means unknown; this lower
bound does not include unobserved historical or older-writer deferrals.
