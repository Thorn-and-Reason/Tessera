# Survey workflow improvements

Implementation brief, 12 October 2026. The user requested implementation of all
six recommendations following the first Instant survey report workflow.

## Outcome and boundaries

Give occasional statistics users a reliable path from a survey export to a
report, with reversible preparation, repeatable reviewed decisions and guided
access to existing inferential analyses. Preserve source columns, files, manual
controls, cancellation, resource budgets and atomic publication. Use synthetic
validation data. All work stays local; no publishing or release promotion.

## Design

Use plain-language reporting actions and short explanations. Show unresolved
decisions first in a stable virtualised review and display how many remain.
Cleaning is optional: the user chooses whitespace trimming, exact label merges
or explicit missing tokens, previews the affected cells, then applies a new
derived column through the existing preparation/undo path. Exact mappings do
not imply case folding. Preview and apply are revision/project bound,
cancellable and budgeted. Recheck roles after preparation.

Save a versioned portable survey recipe separately from the project. Capture
source names/types, approved reporting roles, preparation and presentation
choices. Never bind cross-import matching to internal column IDs. Check the
ordered source structure before replay; mismatches and ambiguous duplicate
names stop automatic reuse and lead back to review. Reapply preparation from
original columns, never from previous derived output. Recipe reads are bounded,
validated passive JSON; writes use the existing protected file transaction.
Project format remains version 3; new preparation operations require explicit
schema support and documented older-reader limits if introduced.

Give generated figures useful variable titles and captions explaining valid
and missing responses. Reuse the existing report palette, editable figure
settings and precision policy. Offer a small fixed set of presentation choices
that recipes can retain. Display rounding must not alter stored estimates,
denominators or accessible exact-value tables.

Guided analysis asks the question, outcome type, independent/paired/repeated
observations and comparison structure before recommending a checked existing
procedure. Open the ordinary form for explicit variable/group/pairing choices.
Unclear, categorical inference, clustered or otherwise unsupported designs
receive an explanation rather than an unsuitable automatic test. No analysis
is run simply because a suggestion was selected.

## Acceptance

Reference fixtures independently specify counts, both percentage denominators,
numeric summaries and edge cases. Prove cleaning preview/apply/undo, original
column preservation, cancellation and stale-result rejection; prove recipe
roundtrip, changed identifiers, structure mismatch, no double cleaning and
untrusted input rejection. Exercise plain-language review, guidance and report
choices through production UI handlers, save/reopen and selected-batch export.
Run full regression, workbook/Python/schema checks and >=80% production
coverage. Native platform/screen-reader acceptance is recorded separately from
automated accessibility tests and cannot be claimed without evidence.

## Execution plan

1. Domain cleaning and its preparation/schema tests; independently review.
2. Passive recipe matching/serialization/protected file I/O; independently review.
3. Report figure defaults and presentation choices; independently review.
4. Guided question flow routing to existing checked forms; independently review.
5. Integrate cleaning preview, recipe replay and simpler survey UI/jobs/state;
   prove atomic publication, undo and retained ownership.
6. Add realistic synthetic reference fixtures and complete numerical/export
   verification; run broad regression and native walkthrough, update this record.

Each owner writes and observes failing tests before implementation. Existing
uncommitted Instant survey report changes are the baseline and must be kept.
Shared modules have one integration owner; independent workers must not revert
one another's changes. Independent correctness/security review precedes delivery.

## Verification

Final local verification passed on macOS ARM64. The application and instrumented
coverage runs completed 1,212 application tests with no failures and two ignored
tests. Production line coverage was 88.77% (37,935 of 42,736 lines). Workbook
worker verification passed 18 tests, Python helper tests passed 35 tests, and
project-schema checks passed 23 tests.

The survey reference fixture run passed all six tests, including the supervised
XLSX comparison. Formatting, Clippy and the release desktop build completed; the
strict warning-as-error Clippy mode still fails on existing project-wide lint
debt, so it is not a release gate for this change. Native walkthrough evidence
covered loading a saved recipe, viewing saved style/cleaning mappings before
apply, replaying to add the new report runs, preserving existing runs and
exporting only the selected survey report batch.

Windows, Linux, real screen-reader speech and compact native-window acceptance
remain separate checks. Existing synchronous ownership bookkeeping is still used
for a few UI admission paths and can reject very large operations conservatively.

## Implemented workflow

Reporting actions now say **Count responses**, **Summarise a measurement** and
**Leave out**. Unresolved actions appear first and retain a stable position
while the user makes choices. Warning explanations are available alongside
the review. Manual analysis and preparation remain accessible.

Optional response preparation trims surrounding whitespace and supports up to
256 exact replacements, including explicitly chosen missing tokens. Several
replacements can be added, removed and previewed together. Trimming precedes
replacement; no case folding or missing-token interpretation is automatic.
At most eight example changes accompany the full affected-cell count. Apply
creates a reviewed column and transfers its reporting action while retaining
the original column. Existing missing rules take precedence. Undo restores
the original dataset; retained analyses remain frozen and are marked stale.

**Save report recipe** stores reviewed choices in a separate `.tessera-recipe`
file before report generation. Files use a closed version-1 JSON envelope and
a 1 MiB limit, with protected file transactions and bounded stamp reads.
Matching requires exact ordered source names and types, independent of column
IDs. Changed structure and already-derived input stop automatic replay.
The loaded recipe shows its saved actions, exact changes and presentation
before **Apply recipe and create report**. That action prepares original
columns and appends a complete report batch atomically, with one undo snapshot.

Report colours and High contrast reuse existing palettes. New figures have
variable titles and captions explaining valid/missing counts and percentage
denominators. Existing two-decimal display retains exact captured values in
accessible report tables. Earlier results and edited figures are preserved.

**Help choose an analysis** asks the aim, outcome type, observation relationship
and comparison structure. Supported recommendations open existing checked
forms for explicit variable choices. Independent multi-group comparisons start
with an empty one-way ANOVA design; repeated comparisons start with an empty
repeated design. Unclear, clustered and unsupported categorical inferential
designs are stopped with an explanation. Guidance never runs an analysis.

## Verification — 12 October 2026

The final application and instrumented suites each passed 1,212 tests with
zero failures. Production line coverage on macOS ARM64 was 88.77% (37,935 of
42,736 instrumented production lines). The workbook suite passed 18 tests;
Python tooling passed 35 and project-schema checks passed 23. All six synthetic
reference cases passed, including supervised XLSX import run explicitly.
Two ordinary-suite entries are deliberately ignored: the separately exercised
XLSX reference test and an allocation-sentinel child invoked by its wrapper.
Formatting, Clippy and diff-whitespace checks passed; existing unrelated lint
warnings remain. Platform-specific files not compiled on macOS are listed in
the coverage summary.

Synthetic CSV/XLSX reference data independently specify category counts, all-
response versus valid-response denominators, sample standard deviations,
constant/all-missing measures, exact label variants, large identifiers and
hostile labels. Tests cover original-cell preservation, declared missing
rules, exact memory-limit boundaries, cancellation/stale publication, undo,
portable recipe binding, structure mismatch, long Unicode names, multiple
replacements, saved settings visibility, guidance design and persistence.

A separate macOS preview imported the synthetic workbook, previewed/applied
a whitespace and exact-label merge, saved a High contrast recipe and generated
the report. A changed import type was refused for recipe reuse. A matching
fresh import reproduced the manually reviewed dataset and figure settings.
All 64 source cells and eight names were preserved; the reviewed Reading count
was five. Native undo/redo, guided independent-comparison routing without an
automatic run, and the rebuilt saved-recipe display were checked. Replaying
onto an existing project preserved all three old analyses and edited figures,
appended runs 4–6, saved a separate project and exported only that new batch.
The HTML retained exact numerical values beside rounded display values and
omitted excluded email responses.

Logs and coverage are retained in `.build/survey-improvements-verification`
and `.build/survey-improvements-coverage-final`; synthetic native outputs are
in `.build/instant-survey-native`. The local preview is ad-hoc signed ARM64,
not notarised or published. No installer or release was promoted.

Native Windows/Linux, screen-reader and compact-window acceptance remain open.
Automated compact layout, zoom, keyboard and accessibility-label tests pass.
Older Tessera readers reject projects containing the new `clean_text`
preparation record; original files are preserved. Existing sharing-aware
memory admission remains synchronous. Retaining original projects and reserving
32 MiB for bounded recipe decoding may reject work conservatively near the
memory limit. Statistical and personal-data suggestions still require review.
