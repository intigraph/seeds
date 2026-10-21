# Repository pendant source

The repository-owned release job exports code and document **pointers**, plus
reviewed summaries of ideas, interfaces, decisions, freshness and starting points.
It does not export source chunk text or line spans. Source witnesses bind each
summary to its defining tracked file. The native index must resolve every
curated pointer before the build can proceed. Stored queries and validating
shapes travel with the share.

`alignments.ttl` is currently a valid empty SSSOM/RDF carrier. Its accompanying
status file explicitly records zero approved mappings and seals those bytes.
This is not evidence of global absence. Missing, changed or malformed input
refuses; an explicit empty carrier does not. Dependencies and glossary references
are not identity alignments. Code/commit deduplication and unrelated exactMatch
records are not cross-repository mapping approval.

Candidate mappings must go through Quipu's existing `align propose` and
`align decide` curation path. Keep their source graphs and source witnesses;
do not fill an approval author from a lexical score. Only the reviewed mapping
record may produce derived identity through the engine's existing apply path.
The current source profile deliberately does not import arbitrary nonempty
mapping data until an actual curated artifact and its transport are reviewed.

The copied release workflow verifies the three public assets only after the
ordinary release finishes. The managed trusted host runs this repository's own
publisher, keeps its publication policy private, pins both native engine
executables and the indexer, and refuses retained uncertain upload outcomes.
Default configuration still publishes only the existing Quipu corpus. Enabling
another repository requires its actual trusted host configuration and a clean
release containing this job; committing this source does not publish an asset.

The publisher requires the engine's model/digest/dimension manifest recipe.
Loading and vector rebuilding belong to the native engine. No hosted job gets
private publication policy or a new credential route. These source controls do
not establish public asset availability, live loading or service acceptance.

## Current delivery boundary

The builder passes the reviewed empty carrier to `quipu share --alignments`,
which seals it as the `alignments.ttl` member. The manifest declares it in
`files.alignments` and `alignments_hash`, and both enter `share_id`. The builder
refuses unless the staged member's bytes equal the reviewed source. The
publisher and the hosted verifier refuse an archive whose member is missing,
undeclared or mis-hashed. The carrier is never appended to graph data.

That contract needs a Quipu engine with the sealed alignment member
(scbrown/quipu#691). An older pinned engine has no such member, so the build
refuses rather than publishing without it. Do not enable this repository's
trusted publication until the trusted host pins an engine that includes it.
Provider-byte embedding recipes and real release assets also require
independent native-engine and publication acceptance.
