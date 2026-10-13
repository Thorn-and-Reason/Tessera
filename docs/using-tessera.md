# Using Tessera

Tessera provides Data, Prepare, Analyse and Results workspaces. Open a
`.tessera` project or import UTF-8 CSV/TSV or Excel (`.xlsx`) data. Use
**Variable details** to set types, measurement levels and descriptions.

## Import and inspect data

For Excel, choose a worksheet and whether its first row contains variable
names, then select **Review worksheet**. The default selects the entire
worksheet when it fits the import limits; a warning identifies a limited
selection for larger sheets. Expand **Custom row and column range** to change
the range and **Column types and names** to adjust inferred columns.

Text identifiers retain their digits by default. Excel may already have
rounded stored numeric values: an explicit acknowledgement covers those source
columns, including when imported as text. Converting a text identifier to a
number requires separate precision approval for that column. Cached formulas
require acknowledgement that their values may be out of date. Formulas without
cached values and Excel's invalid 1900 leap day need separate choices to become
missing. Dates and durations import as ISO text. The importer does not execute
formulas, macros or external links. Only `.xlsx` is supported; `.xls` and
`.xlsm` are excluded.

Select a cell and use arrow keys to move, Enter to edit, Tab or Shift+Tab to
advance, and Escape to cancel an edit. Drag a column header's right edge to
resize it. **Inspect cell** exposes the exact value; Copy preserves numeric
precision. TSV paste validates the complete rectangle before changing existing
cells, with a limit of 4,096 cells and 512 KiB. Blank numeric fields become
missing values; blank text fields remain empty text.

Variable search includes names and descriptions. Unapplied properties require
an Apply, Discard or Stay decision before changing variables, adding a variable,
saving or moving to another workspace.

## Create a survey report

Choose **Instant survey report** from Data or Analyse. Review the proposed
reporting actions and findings, resolve unresolved variables and confirm
numeric measurement roles. **Count responses** produces category frequencies;
**Summarise a measurement** produces numeric summaries and a histogram;
**Leave out** retains the variable in the dataset but omits it from the report.

Optional preparation trims surrounding whitespace and applies explicitly
chosen exact replacements, including missing tokens. Trimming precedes
replacement; case folding is not automatic. Preview the affected-cell count
and before/after examples, then apply a reviewed column while retaining the
original. Existing declared missing rules take precedence. Undo restores the
earlier dataset. Preparation does not silently discard rows or remove outliers.

**Save report recipe** captures reviewed reporting roles, preparation and
presentation in a separate `.tessera-recipe` file. Reuse requires matching
ordered source names and types. The loaded recipe shows its saved settings
before **Apply recipe and create report**. Replay starts from original columns
and appends a complete report batch while preserving earlier results.

Choose **Create report** after review, then edit its ordinary results and
figures. Report colours and High contrast provide presentation defaults.
Captions explain valid/missing counts and percentage denominators.
**Export survey report** selects the generated batch. This temporary batch
selection is not saved in the project; after reopening, use the ordinary saved
history and report-export controls.

The automatic workflow is descriptive. It does not infer repeated measurements,
split multiple-answer cells, score questionnaires or choose inferential tests
without your decisions. Read [reporting rules](design/instant-survey-report.md)
and [recipe and preparation details](design/survey-improvements.md).

## Prepare data

Open **Prepare** to compute a numeric column, recode exact values, sort rows
stably or keep rows matching a condition. These operations retain validated
preparation records. Filtering shows its condition before Apply and the
resulting row count afterwards; Undo is immediately available. Manual edits
retain coalesced descriptive records rather than replayable edit payloads.

For domain scores, use **Prepare → Domain average**, select 2–64 numeric items
and name the new variable. A respondent receives a score only when every
selected item is present. Recode reversed items before averaging. These
descriptive averages assume equally spaced codes and do not establish scale
validity.

## Choose and run analyses

The **Analyse** page offers **Guided** and **Advanced** for all twelve
procedures. Both modes show searchable questions and method descriptions.
Guided keeps detailed options under **More settings**; Advanced displays them
directly. Switching preserves drafts, saved results and running jobs. The
preference is remembered across launches and is separate from saved projects
and statistical recipes.

**Help choose an analysis** asks about your question, outcome type and whether
observations are independent, matched, repeated or clustered. Supported choices
open existing forms for explicit variable selections. Unclear or unsupported
designs receive an explanation. Selecting a recommendation never runs a test.

The procedure catalogue sits beside the form on larger windows; Run stays
visible when the form scrolls. Analysis requirements appear beside Run, and the
run shortcut works only from Analyse. Successful analyses open Results
automatically. Select a saved result, use **Edit selected analysis** to reuse
its settings, or choose **New analysis**. Saved results identify their
procedure, variables and data status.

### Statistical scope

Descriptive statistics include mean, sample standard deviation, variance,
minimum, maximum, sum, range and standard error, with typed filters,
missing-value rules and integer frequency weights. Frequency weights represent
repeated observations. Pearson uses pairwise complete observations by default,
with a separate complete-case option. One-sample, paired and independent t-tests
provide two-sided inference and 90%, 95% or 99% confidence intervals.
Independent comparisons use Welch by default; pooled inference requires an
explicit choice. Ordered roles determine the sign of differences.

Linear regression is available under **Analyse → Explore relationships**.
Choose one numeric outcome, ordered numeric or categorical predictors, and an
explicit reference level for each factor. Fits include an intercept and use
complete cases and optional integer frequency weights. Results retain
coefficients, 90%, 95% or 99% confidence intervals, fit statistics, residual
plots and exact diagnostic tables. Unavailable fits retain their reasons.
ANOVA and scale reliability are also available through their checked forms.

Frequency tables retain every category and distinguish exact text values.
Histograms count the complete eligible sample. Scatterplots retain full-cohort
axis ranges and at most 2,000 deterministic drawing points. Boxplots use
frequency-replicated type-7 quartiles and observed whiskers within the 1.5-IQR
fences. Edits preserve earlier results and mark them stale.

## Edit figures and export reports

Select a saved run in **Results**, expand **Figures**, choose its graph and
select **Edit figure**. The live preview supports titles, captions, axis limits,
typography, palettes, gridlines, marker appearance and physical dimensions.
**Apply figure** saves presentation separately from the captured analysis;
save the project to retain it.

**Export figure…** defaults to PNG. **Apply and export…** and batch export also
start with PNG; SVG remains selectable for vector output. PNG uses the selected
resolution (300 DPI by default) and includes resolution metadata. SVG text is
outlined with the bundled font. Frequency figures offer horizontal and vertical
bars with counts or explicit valid/all-response percentages.

Use **Reuse styles / batch export** to save a named style, apply it to selected
figures, or export the selection with stable filenames. Batch exports use
applied settings. Duplicate a figure to keep an alternative presentation of
the same result. The HTML report uses the same figure renderer and retains
exact tables. **Export all results…** includes every saved run.

Report tables use readable numeric precision; focus or hover a number to inspect
its exact captured value. P-values retain dedicated precision near common
thresholds. Dense labels produce an explanation asking for more space rather
than dropping categories. Older reduced descriptive captures without saved
extrema require a new run with minimum and maximum selected before figure
export.

### Survey charts and group means

For multi-question survey charts, run **Frequencies** with the questions
selected. Choose a **Grouping variable (optional)** to compare groups within
each question. In **Results → Figures → Combined Likert chart → Edit figure**,
confirm the three-, five- or seven-category coding and its neutral midpoint.
Counts and percentages can appear together; every question/group uses its own
valid-answer denominator. Unconfirmed response scales are not included as
agreement charts in reports. Captured variable descriptions provide readable
question labels.

Run **Boxplot** on a domain variable with a grouping variable, then choose
**Group means** in Figures. Grouped boxplots use a shared outcome scale. Means
use the complete captured cohort rather than quartiles or sampled points.
Earlier boxplots without captured means require rerunning.

## Save projects, undo and recover

### Export data safely

CSV/TSV export preserves the original text, including variable names. Tessera
asks for confirmation when text could be interpreted as a spreadsheet formula.
Opening that file directly in Excel or another spreadsheet can evaluate the
text or turn it into an active link; CSV quotation marks do not prevent this.
Use the spreadsheet's text-import command and set the imported columns to
**Text** before loading the file. Check the import preview before proceeding.
Cancel the export if you cannot use a suitable text-import route.

Numeric values remain numeric in the export. Tessera does not add apostrophes
or otherwise change respondent text to suppress formulas. Save a `.tessera`
project when you need to retain the full dataset and its metadata.

### Project files and recovery

Save a `.tessera` project to retain datasets, metadata, captured analyses,
figures and provenance. CSV/TSV export cannot preserve every project metadata
or missing-text distinction. Projects use format version 3 and reject other
versions. Opening a project validates its captured content without replaying
preparation records. Earlier readers reject projects containing the new
`clean_text` records even though the version remains 3; keep original projects
when sharing with an older reader. See the [project format](project-format.md).

**Undo** and **Redo** share a maximum of 32 session history entries within the
data budget. They restore data with a fresh revision and preserve retained
analyses, figures and issued column identities; earlier analyses remain stale.
History stacks are not saved in the project.

Private crash checkpoints run through the single worker after two seconds of
idle time following a dirty change. Recovery offers explicit Open or Discard
decisions; it never marks the user's project file saved. A failed checkpoint
or retirement keeps the editing session available and offers a retry or a
decision to keep the checkpoint. Recovery uses per-user folders rather than
the executable directory. Checkpoints do not replace explicit project saves.

Recovery is implemented on macOS and Linux. Windows refuses to initialise its
recovery store because directory durability has not been verified; editing
and explicit project saves remain available. Do not rely on automatic recovery
on Windows. Verified Linux graphical delivery, AppImage, portable user-data
mode and replay of manual edit summaries remain unfinished. Workbook memory
accounting limits requested Rust heap and retained work; it does not cap total
process resident memory (RSS).

## Appearance and navigation

The workspace supports light, dark and system appearance, with interface
scaling in **View**. Compact windows keep workflow navigation above the
workspace. On macOS, Command-1 through Command-4 follow Data, Prepare, Analyse
and Results; Command-5 opens Variable details.

For builds, packaging and Python verification, see
[build and distribution instructions](build-and-distribution.md). For platform
limits and remaining acceptance checks, see
[production readiness](production-readiness.md).
