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
