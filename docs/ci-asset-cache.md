# Pinned Quipu archives in CI

The remote matrix restores an Actions cache keyed by the exact Quipu tag and
SHA256. Acquisition tries an optional `CI_ARTIFACT_MIRROR_URL` repository variable,
then the verified cache, then the original GitHub release URL. The mirror object
layout is `<base>/sha256/<pin>/<filename>`; no new credential is used.

Every selected archive is hashed before publication and checked again before
extraction. Corrupt bytes or access refusals fail closed. Verified bytes are saved
before the remote tests, so a later test failure does not discard the archive.
A cold cache still requires one successful verified acquisition. No test skip,
server-version change, retry authorization or upstream-recovery claim follows
from enabling caching. Remove the optional variable to disable the mirror;
revert the workflow steps to remove the dedicated cache layer.

The generic downloader is shared byte-for-byte with Quipu's pinned acquisition
helper. Run `python3 scripts/test-ci-asset-mirror.py` for its offline controls.
