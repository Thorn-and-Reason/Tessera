# Tessera by Thorn & Reason

*Quantitative analysis made approachable.*

Tessera is a desktop application for analysing spreadsheet data and producing
tables, editable figures and reports. Guided workflows help people who need a
quick survey summary or occasional statistical analysis, while Advanced mode
keeps detailed analysis and presentation controls available.

**Development status:** the current source is a development candidate.
Production acceptance is still pending. This public repository provides source
code; official executable downloads are not published here. Paid official
packages are planned, subject to release acceptance.

## What you can do

- Import Excel (`.xlsx`), UTF-8 CSV/TSV or saved `.tessera` projects.
- Create descriptive survey reports with **Instant survey report**, after
  reviewing the variables and reporting actions.
- Preview optional whitespace trimming and exact response replacements,
  preserve original columns, and undo applied preparation.
- Save reusable survey recipes containing reviewed reporting, cleaning and
  presentation choices for matching future imports.
- Use **Help choose an analysis** to identify a supported procedure from the
  question and study design, then complete its ordinary analysis form.
- Run descriptive statistics, frequencies, correlations, t-tests, linear
  regression, ANOVA and scale reliability, with histograms, scatterplots and
  boxplots.
- Edit figure titles, captions, axes, typography and colours; use report or
  high-contrast styling, reuse styles and export figures as PNG or SVG.
- Save data and captured analyses together, export data as CSV/TSV, and create
  HTML reports with tables and figures.

**Guided** and **Advanced** use the same analysis procedures. Switching modes
preserves drafts, saved results and running jobs; the preference is remembered
across launches. The app supports light, dark and system appearance and
interface scaling.

## Start with a survey

1. Choose **Instant survey report** from Data or Analyse and import your data
   if needed. For Excel, select and review the worksheet first.
2. Review each variable: **Count responses**, **Summarise a measurement** or
   **Leave out**. Resolve warnings and confirm numeric measurements explicitly.
3. If preparation is needed, preview the changes before applying a reviewed
   copy. You can save the approved settings as a separate `.tessera-recipe`.
4. Choose **Create report**, review its tables and edit its figures through the
   normal controls, then use **Export survey report** for that report batch.

A saved recipe can prepare matching data and generate a report in one action.
Reuse requires the same ordered source column names and types. Mismatches
return you to review. Cleaning is explicit: it does not silently delete rows,
fill missing values, remove outliers or infer questionnaire scores.

The automatic report is descriptive. The analysis guide opens a checked form
for your variable choices; it never runs an inferential test automatically.
Unclear and unsupported study designs receive an explanation. Suggestions
about reporting roles or possible personal data still require your review.

## Work with full controls

Use **Data** to inspect values and variable definitions, **Prepare** to create
or recode variables and select rows, **Analyse** to choose methods and run
analyses, and **Results** to review saved runs and edit their figures.

Existing analyses retain their captured data and settings when you edit the
dataset; changed data are marked so you can decide whether to rerun them.
Figure presentation is saved separately from the numerical results. Rounded
display values retain their exact captured values for inspection and export.

See the [user guide](docs/using-tessera.md) for detailed import, preparation,
survey-chart, figure, keyboard, saving and recovery instructions.

## Build from source

Install Rust 1.95 or newer and your platform's compiler/build prerequisites,
as described in [build and distribution instructions](docs/build-and-distribution.md).
From the repository root:

```sh
cargo build --locked --release --manifest-path crates/tessera-workbook/Cargo.toml --target-dir target --bin tessera-workbook-worker
cargo run --locked --release
```

The workbook worker must remain beside the application for Excel import.
Development packages include both executables. The linked instructions cover
dependency fetching, platform requirements, packaging and verification.

## Verification and current limits

[Verification of the survey workflow](https://github.com/Thorn-and-Reason/Tessera/actions/runs/38296529525)
passed on macOS, Windows and Ubuntu for the tested survey workflow revision,
including dependency and minimum-Rust checks. The application suite passed
1,212 tests, and macOS production line coverage was 88.77%. A macOS walkthrough
also checked import, preparation, recipe reuse, undo/redo, save/reopen and report
export. These records apply to the tested revisions; each new build needs its
own checks.

Clean-machine installation, actual screen-reader speech, complete native
Windows/Linux GUI acceptance and downloaded-file acceptance remain open.
Mac development packages are ad-hoc signed and not notarised; Windows packages
have no publisher signature. Windows crash recovery is disabled, so save
projects explicitly. See [installation guidance](docs/INSTALLING.md) and
[production-readiness requirements](docs/production-readiness.md).

Projects use format version 3. Projects containing the new `clean_text`
preparation records require an updated Tessera reader, even though the format
version is unchanged. Keep original project files when sharing with an older
reader. See the [project format](docs/project-format.md) and
[JSON Schema](docs/project.schema.json).

Only `.xlsx` workbooks are supported; `.xls` and `.xlsm` are excluded. The
importer does not execute workbook formulas, macros or external links.
Resource limits can refuse large imports or operations conservatively.

## Licence and source archives

The original application, workbook worker and build scripts are available
under [GPL-3.0-only](LICENSE). You may build, modify and redistribute them,
including compiled copies, subject to that licence. Third-party dependencies
and fonts retain their own notices. Using Tessera does not require you to
license your ordinary datasets or analysis results under the GPL.

[Versioned source archives and checksums](https://github.com/Thorn-and-Reason/Tessera/releases)
identify specific source snapshots and may predate the current `main` branch.
Distributed binaries must be accompanied by access to their exact corresponding
source. See [licensing and distribution](docs/LICENSING.md).

The [documentation index](docs/README.md) links the project contract, build
instructions, user guide and development records.
