# Instant survey report implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Create editable descriptive survey tables and graphs through one reviewed, data-preserving workflow.

**Architecture:** A survey module checks the current dataset and generates ordinary analysis recipes. Existing workers compute a private candidate project; desktop state publishes it atomically and remembers generated run IDs for selective export. Existing import, numerical engines, figure editing, persistence and file transactions remain authoritative.

**Tech stack:** Rust 1.95, eframe/egui, existing Tessera analysis and ownership-budget APIs; no added dependency.

**Spec:** [Approved design](instant-survey-report.md).

## Global constraints

- Preserve dataset values, row count, column definitions and revision.
- No new dependency, analysis-result type or project schema.
- Numeric summaries require explicit confirmation or existing Scale metadata.
- Suggest Categories only for columns with at most 30 distinct nonmissing values.
- Personal-field heuristics are advisory; show every included column in review.
- Do not infer Likert order, split multiple answers, score scales, impute, deduplicate, select inferential tests or apply survey weights.
- Cancellation, stale results, validation and budget failures publish no partial analyses.
- Charge retained inputs, review findings, scratch indexes and accumulated results before allocation; scans run off the UI thread.
- Use synthetic fixtures and retain at least 80% production coverage.
- Apply Monty writing to new interface and documentation prose.

## Review focus

- Long/hostile spreadsheet headers: bounded display, escaped export, no crashes.
- Empty, constant and all-missing columns: explicit status and honest unavailable estimates.
- An earlier survey batch followed by another: export only the selected batch, retain all historical runs.
- Changed data or pending edits: resolve edits and invalidate the review before computation.
- Personal information with innocuous headers: inspect values too, while explaining that detection is incomplete.

## File ownership

- `src/survey.rs` and `src/survey/`: checked survey contracts, inspection and report generation; split inspection and generation if needed to keep files focused.
- `src/lib.rs`: export the survey module.
- `src/jobs.rs`, `src/jobs/execute.rs`: survey worker contracts, accounting and execution.
- `src/desktop/survey.rs`, `src/desktop/state/survey.rs`: review UI and guarded survey transitions.
- `src/desktop/state.rs`, `state/operations.rs`, `state/accounting.rs`, `state/undo.rs`: survey state lifecycle, retained ownership and publication.
- `src/desktop/mod.rs`, `data.rs`, `analyses.rs`, `results.rs`, `dialogs.rs`: entry points, review rendering and batch export.
- `tests/survey.rs`, `tests/workflow/survey.rs`, `tests/desktop/survey.rs`: focused numerical, state and UI verification; register modules in existing test roots.
- `docs/README.md`, `docs/design/instant-survey-report.md`: user workflow reference and actual verification evidence.

## Task 1: Checked survey review

**Interfaces:** `SurveyRole { Unresolved, Exclude, Categories, NumericMeasure }`; `SurveyChoice { column: ColumnId, role: SurveyRole }`; `SurveyReview { revision: RevisionId, columns: Vec<SurveyColumnReview>, duplicate_rows: usize }`; each column review holds ID, suggested role, valid/missing counts, distinct-value count capped at 31 (display 31 as “more than 30”) and typed findings without private response examples. `SurveyRequest { revision: RevisionId, choices: Vec<SurveyChoice> }` exposes controlled validation and owned-byte accounting.

`inspect_with_budget(dataset: &Dataset, control: &JobControl, retained: &[&Dataset], extra_bytes: usize) -> AppResult<SurveyReview>` is the single inspection entry point. `SurveyReview::owned_bytes() -> AppResult<usize>` charges all returned storage. `SurveyRequest::validate(dataset: &Dataset, control: &JobControl) -> AppResult<()>` rejects missing/duplicate IDs, unresolved roles, nonnumeric numeric measures, a stale revision and zero included supported variables. Omitted IDs mean excluded.

- [x] Write failing tests `forms_columns_require_honest_roles`, `cardinality_boundary_is_30`, `quality_findings_do_not_change_data` and `checked_request_rejects_invalid_roles`. Assert numeric Unknown is unresolved; Scale is NumericMeasure; ordinary short categorical text suggests Categories; name/email/ID/time/free-text candidates are excluded; 30 versus 31 values follows the spec. Include email values under a neutral header, explicit missing rules and precision-sensitive identifiers.
- [x] Run `cargo test --locked --all-features --test survey`; record the expected missing API failure.
- [x] Implement the contracts and cancellable inspection. Count distinct values only up to 31 for suggestions; finish valid/missing and quality scans. Count exact duplicate rows over all columns without changing them, verifying equality after any hash match. Keep explicit missing rules separate from candidate tokens. Record whitespace/case variants and possible multi-answer delimiters as advisory findings. Precharge bounded indexes; resource exhaustion returns an error rather than an incomplete exact duplicate count. Empty/all-missing columns suggest Exclude.
- [x] Add `inspection_cancel_and_budget_leave_input_intact` and `long_headers_and_all_missing_are_bounded`; run the survey tests to PASS.
- [x] Review the task for unsupported semantic inference and uncharged allocations before proceeding.

## Task 2: Atomic descriptive report generation

**Interfaces:** `generate_with_budget(project: &Project, request: &SurveyRequest, control: &JobControl, retained: &[&Dataset], extra_bytes: usize) -> AppResult<Project>`. `extra_bytes` contains caller-owned storage outside the input project and request; this function charges those inputs once itself. New run IDs are the contiguous suffix after the input project's run count.

- [x] Write failing `batch_matches_manual_counts_and_summaries`, `batch_preserves_dataset_and_prior_runs`, `batch_is_atomic_on_failure`, and `multiple_answers_require_an_explicit_role`. Assert both percentage denominators, missing N, sample SD and known histogram totals against independently calculated fixture values.
- [x] Run `cargo test --locked --all-features --test survey`; require RED on generation tests.
- [x] Generate one Frequencies request for categorical columns, one Describe request for numeric measures (Mean, SampleStdDev, Minimum, Maximum; PerVariable missingness), and one Histogram request per numeric measure with Automatic bins. Use no filter and no weight. Reuse `analysis::analyze_with_budget` and `Project::with_run_with_budget_controlled`. Charge the original project and every accumulated candidate while results and temporary shells coexist. Never discard categories.
- [x] Skip meaningless plots for empty/all-missing measures; retain missing counts and unavailable-estimate explanations through the existing engine. Existing Describe distribution figures remain available alongside histogram figures; do not change global manual figure behavior. Add one concise provenance entry recording included roles, excluded IDs and unresolved quality findings without copying response values. Precharge the audit string before construction.
- [x] Add cancellation, near-limit storage and save/reopen tests; run survey tests to PASS. Confirm unchanged cells, metadata, revision and earlier run identities.
- [x] Review numerical and ownership invariants before proceeding.

## Task 3: Worker and guarded desktop state

**Interfaces:** add `JobOperation::InspectSurvey { project: Arc<Project> }` and `JobOperation::SurveyReport { project: Arc<Project>, request: SurveyRequest }`; add distinct `JobOutput::SurveyReview(Arc<SurveyReview>)` and `JobOutput::SurveyReport(Arc<Project>)`. `DesktopState` owns an optional review, checked choices, entry/import intent and generated run IDs. Public survey entry/start methods supply the testable state API; keep implementation in `state/survey.rs`.

- [x] Write failing state/worker tests for guarded entry, review completion, stale/cancelled completion, revision invalidation, another imported document, budget limits and failed batch publication. Assert no survey job starts from an old project Arc or while cell/property edits remain unresolved.
- [x] Run `cargo test --locked --all-features --test workflow survey`; require RED.
- [x] Extend `JobOperation::project`, `pending_bytes_controlled`, worker execution, desktop operation labels/recipe accounting and publication. Use existing token checks. Charge survey review/choice/run-ID storage in retained UI accounting and copies. Survey output must not use Prepared adoption, which has data-preparation semantics.
- [x] Preserve import intent across the existing preview/confirmation flow; start inspection only after a successful import. Cancel/error clears or restores intent predictably. Project adoption clears survey state; revision changes invalidate both review and temporary batch export selection. Navigate through existing property/cell guards without triggering a document-replacement warning just to open survey review.
- [x] On success, validate the candidate's source revision and prior-run prefix, publish once, clear stale frequency caches, select the first new run and open Results. Track precisely the new run-ID suffix; preserve earlier analyses and dirty-state behavior.
- [x] Run workflow survey tests to PASS and review all exhaustive callers and clone/reset paths.

## Task 4: Review, editing and selective report export

**Interfaces:** `desktop::survey::show(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner)` renders the review through the existing Analyse surface. Add a survey-specific export route using the saved generated run IDs and the existing ExportReport operation; ordinary report export continues to include normal history.

- [x] Write failing desktop tests for entry from empty Data and Analyse, review roles/warnings, disabled invalid generation, keyboard focus, compact 640 × 440 review scrolling and Results export reachability. Add export integration tests proving a second batch excludes the first batch and hostile labels are escaped.
- [x] Run `cargo test --locked --all-features --test desktop survey` and the focused workflow tests; require RED.
- [x] Add **Instant survey report** entry actions. Render valid/missing counts, role suggestions, include/exclude controls, warnings and links to existing preparation controls in a virtualised review. Unresolved fields require a decision, including the explicit complete-cell category choice for multiple answers. Use a fixed reachable footer for **Create report** and a way back to manual analysis. Both Guided and Advanced retain access.
- [x] Add **Export survey report** on Results when a batch selection exists. Validate selected IDs and precharge their copies before opening export. Reuse existing file dialog and atomic report writer. Keep ordinary result/figure editing available.
- [x] Run focused UI/workflow/export tests to PASS; review accessible labels, pending-edit handling and personal-data defaults.

## Task 5: Complete verification and delivery

- [x] Run `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features`, `cargo test --locked --all-features`, and `cargo test --locked --manifest-path crates/tessera-workbook/Cargo.toml`; investigate every new failure or warning.
- [x] Run `python3 scripts/coverage.py --output .build/survey-coverage`; require production coverage ≥80% and retain the summary. Run repository schema checks if persisted-result validation tests indicate a regression; no schema migration is expected.
- [x] Perform a native macOS walkthrough using a synthetic Forms-style workbook: import, review, generate, edit a figure, save/reopen, export a selected survey batch. Check compact layout and keyboard operation. Compare exported values with fixture expectations.
- [x] Have a fresh code reviewer assess correctness/data preservation and a security reviewer assess value exposure, escaping, budgets and stale publication. Address critical/high findings and rerun only affected verification.
- [x] Update the existing documentation index and design verification section with actual results and limitations. Preserve the supplied spreadsheet; do not promote or publish a release, push changes or modify third-party resources without separate authorization.

## Execution handoff

Use subagent-driven execution, consistent with the supplied Agent-First instructions. Assign each task its owning modules and tests, warn workers that others share the checkout, and keep tightly coupled state/UI work sequential. Review each completed task before starting its dependent task; perform a final whole-change review.

Implementation and local verification completed on 12 October 2026. Native compact resizing, Windows/Linux and screen-reader acceptance remain open; see the design verification record. No release was published.
