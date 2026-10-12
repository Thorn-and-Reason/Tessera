# Tessera workflow redesign

The desktop workflow must let a new user import a spreadsheet, inspect and prepare data, choose an analysis, assign variables, run it, and find and export the result without discovering hidden inspectors or duplicated output screens.

## Design

- Data shows the worksheet, import and editing actions, and explicit routes to variable details, preparation, and analysis.
- Prepare is a workspace for the existing Compute, Recode, Sort, and Filter operations. Explain each operation and its effect before applying it. Existing undo, revision, validation, and worker safeguards remain authoritative.
- Analyse is a full-page builder. Show task-based procedure discovery with descriptions, then variable roles and options, with the Run action reachable in small windows. All twelve procedures remain available; Guided and Advanced affect detail, not access.
- Results is the only output workspace. Show saved analyses, their source-data status, a route to edit their settings, a route to create an analysis, and report export. Reports still include all saved results.
- Navigation presents Data, Prepare, Analyse, Results in order. Variable details remain directly accessible from Data and navigation. Compact windows use compact navigation rather than spending worksheet width on a fixed sidebar.
- Retain existing fonts, accessible focus/contrast, keyboard shortcuts, project format, analysis engine, import limits, and bounded/virtualized lists. No new dependency.

## Implementation plan

1. Add failing UI and state tests for visible preparation, compact navigation, a full-page analysis builder, separate result history, and automatic result navigation.
2. Extend the existing Workspace/state action layer with Prepare and guarded transitions. Preserve unapplied metadata via the existing Apply/Discard/Stay decision and resume the requested workspace only after resolving it.
3. Redesign the three workflow surfaces in their existing desktop modules. Keep checked drafts and existing operation submission paths.
4. Integrate navigation, reduce top-level chrome, and keep primary actions reachable at 640 × 440 and larger sizes. Test zoom and keyboard focus.
5. Test the supplied workbook locally through the workbook worker, analysis, project save/reopen, and report export. Do not copy private survey values into tracked fixtures or documentation.
6. Run desktop/unit/integration suites, format and lint checks, production coverage, and an actual native walkthrough. Review the final changes for data loss, state races, and inaccessible controls.

## Acceptance and review focus

- Import and prepare remain accessible with an empty dataset and after an import.
- A pending cell or metadata edit cannot be silently lost when navigating.
- Cancelled, stale, or failed worker operations do not change the selected workspace or current project unexpectedly.
- Guided and Advanced both expose all twelve procedures and preserve drafts.
- Analysis setup, Run, saved results, and export remain usable in compact windows.
- Real workbook import, a numeric summary and relationship analysis, save/reopen, and HTML export succeed with original data preserved.

The user authorised proceeding with the complete redesign in this chat. This plan implements that authorised workflow within the existing native app; publishing and release promotion are separate actions.

## Verification

The final macOS run passed 1,080 application tests and 18 workbook-worker tests, with one ignored application test. Production-source line coverage was 88.49% (33,749 of 38,139 instrumented lines). Formatting, Clippy and diff checks passed; existing dead-code and test-helper warnings remain. This change introduced no dependency.

The native Apple Silicon preview imported the supplied survey workbook (75 rows, 12 variables), accepted the explicit grouped stored-number acknowledgement, ran Descriptives, sorted and undid the data, ran Pearson, saved a separate project, and exported both results. Every imported column and value in the saved native test project matched the independent workbook probe after Undo. Native View-menu navigation also opened Prepare. Private workbook contents and generated projects/reports remain in ignored local build outputs.

Layout/input tests cover compact and large windows, zoom, all twelve procedure footers, catalog/variable reachability, result scrolling, and macOS header dragging. Native review exposed and fixed the compact Results overflow and the analysis form's hidden variable choices.

The preview is an ad hoc signed Apple Silicon development build. It has not been notarised, promoted as a release, or tested natively on Windows/Linux. Native Windows/Linux visual checks, clean-machine installation and screen-reader acceptance remain open. The existing macOS archive script's icon-generation step failed during preview packaging; the local preview reused the icon from the existing development candidate. That packaging issue remains outside the workflow change.
