# Seed terminology in agent help

Trigger: request `sd --help` or `sd cutover --help`.
The installed baseline introductory sentence uses the retired prose term;
the candidate introductory sentence calls seeds the work-item tracker, and
cutover help calls its input legacy JSONL. Commands, flags and wire names
are unchanged. This is a candidate demonstration, not installed acceptance.

The baseline is the installed `sd`; its exact source revision is unverified.
The candidate is an actual local build of this proposal on source base
`e7f52ef591a8c4a8878cee221fcfde59a045ad35`. Its CLI source SHA256 is
`012b90151ff11069bf4a3de2b42a2ab32e1a9c2281199319b684df35750acf6d`.

The small [output recording](output.txt), [timing](timing.txt) and
[repeat script](run-demo.sh) are retained here with the proposal, following
this repository's existing `docs/probes/pr-demos` convention. The output
also serves as the readable transcript. It retains the literal baseline
wording as evidence; it is not a current instruction.

Download both recording files and replay with:

```sh
scriptreplay --log-out output.txt --log-timing timing.txt
```

Replay was verified locally. The recording is terminal output, not an inline
video. To repeat, set `BASE_SD` and `CANDIDATE_SD` to the two executables and
run `bash run-demo.sh`; no ledger, credentials or network service is needed.
