#!/usr/bin/env bash
set -euo pipefail
: "${BASE_SD:?path to the installed baseline sd}"
: "${CANDIDATE_SD:?path to the candidate sd}"
printf 'Installed baseline: sd --help (source revision unverified)\n'
"$BASE_SD" --help | sed -n '1p'
printf '\nCandidate source: sd --help\n'
"$CANDIDATE_SD" --help | sed -n '1p'
printf '\nCandidate source: sd cutover --help\n'
"$CANDIDATE_SD" cutover --help | sed -n '1p'
