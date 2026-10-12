# Tessera

A Thorn & Reason application.

**Status: local development candidate, not approved for public production release.**
Use the [documentation index](docs/README.md) and
[production-readiness requirements](docs/production-readiness.md) for remaining
release gates. The original application and workbook-worker code use
[GPL-3.0-only](LICENSE); see [licensing and distribution](docs/LICENSING.md).

See [build and distribution instructions](docs/build-and-distribution.md) to
build from source or prepare packages. Official prepared packages will be sold
through [Thorn & Reason](https://thornandreason.com/tessera/); this public
repository publishes source rather than official executable downloads.

The **Analyse** page offers **Guided** and **Advanced** for all twelve procedures.
Both modes show searchable questions and method descriptions. Guided keeps
detailed options under **More settings**; Advanced displays them directly. Switching preserves the current draft, saved results and
running jobs. The preference is remembered across launches and is separate
from saved projects and statistical recipes.

A native Rust workspace for data, descriptive statistics, Pearson correlations, linear regression, ANOVA, scale reliability, t-tests, frequencies and charts, with the Thorn & Reason interface.

Open a `.tessera` project or import UTF-8 CSV/TSV or Excel (`.xlsx`), then follow the workspace navigation:

1. **Data**: import and check values. Use **Variable details** to set types, measurement levels and descriptions.
2. **Prepare**: compute or recode variables, sort rows, or keep rows matching a condition.
3. **Analyse**: search by question or method, select variables, and run the analysis. The procedure catalog sits beside the form on larger windows; Run stays visible when the form scrolls.
4. **Results**: successful analyses open here automatically. Select a saved result, use **Edit selected analysis** to reuse its settings, or choose **New analysis**. **Export all results…** creates an HTML report containing every saved run.

Save projects in the documented open JSON format; export data as CSV/TSV. Compact windows keep workflow navigation above the workspace. On macOS, Command-1 through Command-4 follow Data, Prepare, Analyse and Results; Command-5 opens Variable details.

For report figures, select a saved run in **Results**, expand **Figures** and choose its graph, and select **Edit figure**. The live preview supports titles, captions, axis limits, typography, palettes, gridlines, marker appearance and physical dimensions. **Apply figure** saves presentation separately from the captured analysis; save the project to retain it. **Export figure…** defaults to PNG, a widely supported image format. **Apply and export…** and batch export also start with PNG; SVG remains selectable for vector output. PNG uses the selected resolution (300 DPI by default). SVG text is outlined with the bundled font, and PNG output includes resolution metadata. Frequency results also offer horizontal and vertical bar charts with counts or explicit valid/all-response percentages.

Use **Reuse styles / batch export** to save a named style, apply it to selected figures, or export the selection with stable filenames. Batch exports use applied settings. Duplicate a figure to keep an alternative presentation of the same result. The report export uses the same figure renderer and retains exact tables. Dense labels produce an explanation asking for more space rather than dropping categories. Older reduced descriptive captures without saved extrema require a new run with minimum and maximum selected before figure export.

Projects use format version 3. This unreleased build rejects other versions; opening a project does not replay its preparation records.

Open **Prepare** to compute a numeric column, recode exact values, sort rows stably or keep rows matching a condition. These operations retain validated preparation records. Filtering shows its condition before Apply and the resulting row count after adoption; Undo is immediately available. Manual edits retain coalesced descriptive records rather than replayable edit payloads.

**Undo** and **Redo** share a maximum of 32 session history entries within the data budget. They restore data with a fresh revision and preserve retained analyses, figures and issued column identities; earlier analyses remain stale. History stacks are not saved in the project.

Private crash checkpoints run through the single worker after two seconds of idle time following a dirty change. Recovery offers explicit Open or Discard decisions; it never marks the user’s project file saved. A failed checkpoint or retirement keeps the editing session available and offers a retry or a decision to keep the checkpoint. Recovery uses per-user folders rather than the executable directory.

Crash recovery is available on macOS and Linux. Windows currently refuses to initialise its recovery store because directory durability has not been verified; editing and explicit project saves remain available. Do not rely on automatic recovery on Windows.

For Excel, choose a worksheet and whether its first row contains variable names, then select **Review worksheet**. The default selects the entire worksheet when it fits the import limits; an explicit warning identifies a limited selection for larger sheets. Expand **Custom row and column range** to change the range and **Column types and names** to adjust inferred columns. Text identifiers retain their digits by default. Excel may already have rounded stored numeric values: one explicit acknowledgement covers those source columns, including when imported as text. Converting a text identifier to a number requires separate precision approval for that column. Cached formulas require acknowledgement that their values may be out of date. Formulas without cached values and Excel’s invalid 1900 leap day need separate choices to become missing. Dates and durations import as ISO text. The importer does not execute formulas, macros or external links. Only `.xlsx` is supported; `.xls` and `.xlsm` are excluded.

The workspace supports light, dark and system appearance, with interface scaling in **View**. Select a data cell and use arrow keys to move, Enter to edit, Tab or Shift+Tab to advance, and Escape to cancel an edit. Drag a column header's right edge to resize it. **Inspect cell** exposes the exact value; Copy preserves numeric precision. TSV paste validates the complete rectangle before changing existing cells, with a limit of 4,096 cells and 512 KiB. Blank numeric fields become missing values; blank text fields remain empty text.

Variable search includes names and descriptions. Unapplied properties require an Apply, Discard or Stay decision before changing variables, adding a variable, saving or moving to another workspace. Analysis requirements appear beside the Run action; the shortcut runs only from Analyse. Saved results identify their procedure, variables and data status, while methods and detailed values remain available for inspection. Grouped boxplots use a shared outcome scale. Report export includes all retained runs.

Version 0.3.0 supports mean, sample standard deviation, variance, minimum, maximum, sum, range and standard error, with typed filters, missing-value rules and integer frequency weights. Frequency weights represent repeated observations. Pearson uses pairwise complete observations by default, with a separate complete-case option. One-sample, paired and independent t-tests provide two-sided inference and 90%, 95% or 99% confidence intervals. Independent comparisons use Welch by default; pooled inference requires an explicit choice. Ordered roles determine the sign of differences.

Frequency tables retain every category and distinguish exact text values. Histograms count the complete eligible sample. Scatterplots retain full-cohort axis ranges and at most 2,000 deterministic drawing points. Boxplots use frequency-replicated type-7 quartiles and observed whiskers within the 1.5-IQR fences. Reports include charts and exact underlying tables; edits preserve earlier results and mark them stale.

The development increment adds linear regression under **Analyse → Explore relationships**. Choose one numeric outcome, ordered numeric or categorical predictors, and an explicit reference level for each factor. Fits include an intercept and use complete cases and optional integer frequency weights. Results retain coefficients, 90%, 95% or 99% confidence intervals, fit statistics, residual plots and exact diagnostic tables. Unavailable fits retain their reasons. Release acceptance requirements are recorded in [production readiness](docs/production-readiness.md).

## Local builds

Rust 1.95 or newer is required. From this directory:

```sh
cargo build --locked --release --manifest-path crates/tessera-workbook/Cargo.toml --target-dir target --bin tessera-workbook-worker
cargo run --locked --release
```

Excel import needs the sibling `tessera-workbook-worker` executable; the first command builds it beside the desktop executable. Packages include both. The Cargo package and desktop executable are named `tessera`. Projects use the documented `.tessera` contract and version policy above. The desktop feature is enabled by default. Core checks and the development benchmark can use `--no-default-features`.

```sh
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features
```

## Packages and release acceptance

[Production readiness](docs/production-readiness.md) records the release gates.
Package `BUILD-INFO.json` identifies the actual build inputs, including any
uncommitted source; `SHA256SUMS.json` identifies every package payload. An
unchanged version number alone does not identify a build.

Earlier native Mac ARM64 checks exercised Excel import, project Save/Open,
mode persistence, dirty Quit cancellation and deliberate crash/recovery using
disposable data. Linux ARM64 container checks exercised the workbook helper
and file-safety tests. These observations apply to the builds tested at the
time; the current source and each prepared package need their own acceptance.
Windows evidence is cross-compilation only. Clean-machine installation,
complete native procedure checks and screen-reader acceptance remain open.

Local macOS packages contain a universal ARM64/Intel application. Windows packages contain an x64 executable. Build them with the installed cross-compilers:

```sh
python3 scripts/package.py --platform macos --output dist/Tessera-0.3.0-macOS.zip
python3 scripts/package.py --platform windows --output dist/Tessera-0.3.0-Windows.zip
```

Linux source includes X11/Wayland rendering, portal dialogs and a Linux-specific safe publication backend. File-safety tests have passed in a Linux ARM64 container; graphical desktop workflows remain unverified. On a native x86_64 or ARM64 Linux GNU host, `python3 scripts/package.py --platform linux --output dist/Tessera-0.3.0-Linux.zip` creates an executable archive. Its system-library dependencies and supported distribution baseline must be checked before distribution.

An existing archive is preserved. Packages include the desktop executable, workbook worker, application licence/distribution notice, current format documentation, public schema, example projects, font licences, third-party notices and build-input identities. These are development packages. Official Mac distribution uses unsigned/ad-hoc packages; downloaded-file acceptance remains required.

See the [current project format](docs/project-format.md) and [schema](docs/project.schema.json). Projects retain datasets, metadata, captured results and provenance. CSV/TSV cannot preserve every project metadata or missing-text distinction; save a `.tessera` project to retain them.

Verified Linux graphical delivery, AppImage, portable user-data mode and replay of manual edit summaries remain unfinished. Crash checkpoints are implemented; they do not replace an explicit project save. Workbook memory accounting limits requested Rust heap and retained work; it does not cap total process resident memory (RSS).

Python helper checks require Python 3.11+ and `scripts/requirements-dev.txt`. Install those development dependencies in a virtual environment, then run `python3 -m unittest discover -s scripts -p 'test_*.py'`.

The current desktop redesign and its verification are documented in [the desktop design system](docs/design/desktop-design-system.md).
