# Tessera desktop design system

The user chose a modern native desktop interface: clean toolbars, compact controls and clear panels. This revision applies desktop design principles across the existing app rather than moving individual controls between screens.

## Skills used

The skills registry search found and the task downloaded these MIT-licensed skills for review and application:

- [UX for Desktop](https://skills.sh/davidortinau/maui-skills/ux-desktop): precise pointer input, higher information density, resize resilience and primary keyboard operation.
- [UX First Principles](https://skills.sh/davidortinau/maui-skills/ux-first-principles): hierarchy, proximity, consistency and progressive disclosure.
- [macOS Design Guidelines](https://skills.sh/ehmo/platform-design-skills/macos-design-guidelines): standard window controls, compact 22–28pt controls, source lists, semantic typography and native menus.
- [Native-Feel Cross-Platform Desktop](https://skills.sh/yetone/native-feel-skill/native-feel-cross-platform-desktop): platform-owned windowing and explicit evaluation of UI/backend boundaries. Its multi-runtime architecture is unnecessary for Tessera's existing Rust analysis engine and current design requirements.

These skills inform the design; examples tied to SwiftUI, React or MAUI do not require adopting those frameworks. Source copies are held in a temporary task folder, not installed as global instructions.

## Framework decision

Retain Rust and egui/eframe. Current problems are visible in the owning source: an 86pt app bar, large navigation blocks, serif interface headings, paragraph-sized procedure buttons and form controls competing with result content. egui already supports layout panels, custom reusable controls, accessible keyboard interaction, fonts and native window/menu integration. Those capabilities cover the selected design. A framework migration would rebuild every desktop surface and risk the checked draft, worker, memory-budget and project-persistence boundaries without addressing a demonstrated blocker.

A web frontend remains an option if later evidence identifies a requirement that egui cannot satisfy economically. It would still consume the Rust engine through a checked typed boundary; no calculation or project format would move into UI code.

## Design decisions

- Standard native window chrome and a single compact app toolbar; document state and frequent actions remain visible.
- Compact source-list navigation with four workspaces and a secondary Variables route. No large instructional cards in navigation.
- White/neutral light surfaces and charcoal dark surfaces, a restrained blue accent, system body font on Mac with bundled multilingual fallback, 13pt body text and 18pt sans headings. Brand typography belongs to the small wordmark, not every page heading.
- 26pt controls, 28pt one-line rows and 40pt two-line result rows. Shared rows retain keyboard activation, focus indication, selection and accessible labels; display text truncates with a tooltip.
- Data gives the worksheet priority. Editing tools fit a compact toolbar; variable details are a dedicated master/detail workspace rather than a hidden inspector.
- Analysis uses a 248pt method source list with a description shown once for the selected method. Variable roles and common options lead the form; advanced controls disclose progressively. Run is a compact right-aligned footer action.
- Preparation uses compact tool choices and a readable bounded form, with history disclosed on demand and Apply/Undo in a stable footer.
- Results use a compact saved-result list and a distinct report surface. Figure editing is secondary disclosure rather than a permanent second toolbar. Exact captured values and export scope remain authoritative.
- Import and decision dialogs use consistent spacing, visible default actions and concise guidance while retaining all precision, conversion and data-loss safeguards.

## Proof

Verify density and visible controls at compact and large sizes, all twelve analysis forms, keyboard focus, saved recipes, long names, data edits and property prompts. Test the supplied workbook locally through import, preparation/Undo, analysis, save/reopen and export. Review native screenshots of the complete rebuilt app, not isolated widgets. Run the existing suite, production coverage, formatting, lint and an independent review before delivery. Private survey contents remain excluded from tracked fixtures and preview packages.

## Verified preview — 12 October 2026

The final macOS run passed 1,080 application tests and 18 workbook-worker tests, with one ignored application test. Fresh production line coverage was 88.49% (33,749 of 38,139 lines), recorded in `.build/native-desktop-verification-final/coverage/summary.json`. Formatting, Clippy and diff checks passed; existing dead-code and test-helper warnings remain.

The native walkthrough imported the supplied workbook (75 rows, 12 variables), inspected all five workspaces, applied and undid a sort, ran Descriptives and Pearson, inspected the figure editor, saved and reopened the project, and exported both analyses. The saved dataset matched the independent workbook probe column-for-column and value-for-value. Light and dark appearances were inspected. Automated UI tests cover all twelve methods, compact navigation, zoom, figure actions, exact result values, paging and allocation limits.

The preview is an Apple Silicon development build with an ad hoc signature. It is not notarised. Native Windows/Linux visual checks, clean-machine installation and screen-reader acceptance remain open. The preview archive excludes the supplied workbook and generated survey projects/reports.

## Figures and export follow-up

Figure export defaults to PNG for compatibility with Office applications, image viewers and publishing tools. SVG remains available in the editor. The default PNG has a white background and 300 DPI; user-selected transparency, physical dimensions and resolution remain respected.

The graph work applies [Diagram Design](https://github.com/cathrynlavery/diagram-design)'s guidance on restrained colour, label hierarchy, clear boundaries and honest data placement to Tessera's existing Noto Sans and blue palette. Its diagram-specific limits on numbers of nodes or points do not justify removing statistical observations or categories. The installed skill was read from the local plugin cache; no global settings or application dependencies changed.

Native and exported descriptive distributions now stack equal exact values, with each dot representing one retained source row rather than frequency-weight replication. Nearby unequal values retain distinct x coordinates; positive and negative zero share a stack. Sampling, missing values and weights remain disclosed. Exported boxplot whiskers connect to their respective quartiles, with separate stems for observed whiskers inside interpolated quartiles. Markers have outlines; extrema remain fully drawn at boundary ticks. Count axes use whole-number ticks, and wrapped vertical labels reserve additional space. SVG titles and descriptions have unique referenced IDs.

These are presentation changes: saved numerical results and project formats remain unchanged. Existing editable figure settings still apply.

Fresh macOS all-features verification after the figure changes passed 1,088 application tests and 18 workbook-worker tests, with one ignored application test. Production line coverage was 88.51% (33,827/38,219 lines), recorded in `.build/figure-design-verification/coverage/summary.json`. Formatting, Clippy and diff checks passed, with existing warnings. The native save dialog created a valid PNG by default; the editor opened with PNG selected and SVG remained selectable. Rendered examples covered distributions, frequencies, histograms, scatterplots, boxplots and regression diagnostics. Windows/Linux visual checks and screen-reader acceptance remain outstanding.

## Stable button geometry

Button borders keep the same width when idle, hovered or pressed. Selectable controls share a borderless layout and paint keyboard focus inside their existing bounds. This prevents hovering or changing a selection from resizing controls and moving their neighbours across toolbars, navigation, forms, dropdowns and dialogs.

Regression tests compare control rectangles before hover, during hover and after the pointer leaves in light/dark themes, wide/compact layouts and at 150% UI scaling. Shared-control tests also cover pressing, changing a selection and visible keyboard focus.

Fresh all-features macOS verification passed 1,090 application tests and 18 workbook-worker tests, with one ignored application test. Production line coverage was 88.50% (33,857/38,257 lines). Formatting, Clippy, desktop build and diff checks passed; existing warnings remain. Independent review found no issues.

## Bulk variable selection

Descriptives, Correlation and Frequencies offer Select all. Descriptives and Correlation select numeric variables; their search fields change the action to Select all matching across every candidate page. Frequencies selects all column types. Regression offers the same action in its predictor picker, excluding the outcome and preserving numeric/categorical roles. Reliability selects numeric items, and factorial ANOVA selects factors excluding its outcome. Bulk selection preserves the existing order and appends eligible variables in dataset order without duplicates.

Reliability's 64-item and ANOVA's six-factor limits disable an oversized bulk action with an explanation. Memory admission precedes allocation and edits, so refusal retains the original selection. Individual role selectors and repeated-measures cell mappings retain their specific assignments.

Bulk-selection verification passed all 569 library tests and 19 focused UI integration tests, including the hover regression. Formatting, diff checks, Clippy and the desktop build passed with existing warnings. Independent review found no actionable issues. Fresh coverage measurement and a native pointer walkthrough were not repeated for this follow-up.

## Report-quality survey output

The supplied survey notebook and PDF provide the visual benchmark: wrapped question labels, consistent response colours, direct values, per-question valid-answer denominators, aligned group comparisons and explicit missingness notes. The implementation retains Rust and extends the existing captured-analysis and shared SVG pipeline.

The work covers combined diverging Likert charts with explicit scale confirmation; captured grouped frequencies; complete-answer respondent domain averages; grouped mean figures from actual captured means; and white, readable statistical tables with compact display precision and access to exact values. Reliability coefficients remain distinct from domain scoring. Domain averages assume equally spaced numeric codes and do not establish scale validity.

Acceptance uses a small reproducible fixture and the private workbook locally: check category counts and denominators against an independent calculation, zero-answer and absent-category handling, grouping, domain missingness, escaped labels, saved/reopened settings, unchanged historical captures, and the same SVG in preview/PNG/report. Check light and dark native output, long labels, print tables and error paths. Private rows, derived projects and rendered report examples stay in ignored build output and outside the PR and preview package.

Fresh all-features macOS verification passed 1,121 application tests and 18 workbook-worker tests, with one ignored application test. Production line coverage was 88.52% (35,076/39,627 lines). All 23 project-schema checks, formatting, diff checks, Clippy and the desktop build passed; existing warnings remain. Chromium print verification confirmed fitting tables/charts stay whole and oversized tables keep all rows and repeated headers. Independent review found no remaining blockers.

The private workbook benchmark retained 73 of 75 submissions under the notebook's stated rating rule. All ten item distributions, staff-group counts, three complete-answer domain scores and school means matched independent calculations. Saved/reopened projects and public PNG/SVG/HTML export paths passed. Native macOS checks reopened the project, inspected the Results table and grouped figure editor, and applied/undid a disposable domain variable. The preview remains an Apple Silicon development build with an ad hoc signature; Windows/Linux visual and screen-reader acceptance checks remain open.
