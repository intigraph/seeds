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

This source profile validates the explicit empty alignment carrier, but the
current native share transport has no sealed `alignments.ttl` member. The
repository release pendant is not content-complete until the native engine
supports that member and its verification contract is wired here. An empty
source file or successful fixture is not proof that the archive carries it.
Do not enable this repository's trusted publication while that requirement
is unresolved. Provider-byte embedding recipes and real release assets also
require independent native-engine and publication acceptance.
