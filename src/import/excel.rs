//! Typed Excel import from a retained, supervised workbook snapshot.
use super::{
    allocate_values, build_columns, check_bytes, choice_bytes, construction_bytes, import_number,
    validate_missing, ImportColumnChoice, ImportColumnPreview, ImportWarning, Values,
};
use crate::{
    data::{Budget, ColumnKind, Dataset, MissingRules, MAX_CELLS, MAX_OWNED_BYTES},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::{borrow::Cow, collections::HashSet, mem::size_of, path::Path, sync::Arc};
use tessera_workbook::{
    metadata::{WorkbookInfo, MAX_METADATA, MAX_SHEETS, MAX_SHEET_NAME},
    protocol::{validate_range, Cell, Value, MAX_OUTPUT, MAX_STRING},
    supervisor::{self, Config, WorkbookSnapshot},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExcelSelection {
    pub worksheet_index: u32,
    /// Inclusive, zero-based row/column bounds, including the optional header row.
    pub range: [u32; 4],
    pub header: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExcelDecisions {
    pub acknowledge_cached_formulas: bool,
    pub allow_missing_formula_caches: bool,
    pub allow_fictitious_dates_as_missing: bool,
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExcelWarnings {
    pub cached_formula_cells: usize,
    pub missing_formula_cache_cells: usize,
    pub fictitious_date_cells: usize,
}
#[derive(Debug)]
pub struct ExcelWorkbook {
    snapshot: WorkbookSnapshot,
    metadata: WorkbookInfo,
    owned_bytes: usize,
}
impl ExcelWorkbook {
    pub fn metadata(&self) -> &WorkbookInfo {
        &self.metadata
    }
    pub fn source_identity(&self) -> [u8; 32] {
        self.snapshot.source_identity()
    }
    /// Includes a conservative reservation for the private compressed source.
    pub fn owned_bytes(&self) -> usize {
        self.owned_bytes
    }
    pub fn from_snapshot(
        snapshot: WorkbookSnapshot,
        metadata: WorkbookInfo,
        control: &JobControl,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        control.checkpoint(0, None)?;
        let owned_bytes = check_bytes(snapshot.owned_bytes(), metadata.owned_bytes())?;
        let owned_bytes = check_bytes(owned_bytes, size_of::<Self>() + 2 * size_of::<usize>())?;
        check_bytes(Budget::dataset_bytes(retained, extra_bytes)?, owned_bytes)?;
        if metadata.source_identity != snapshot.source_identity()
            || metadata.sheets.len() > MAX_SHEETS
            || metadata.owned_bytes() > MAX_METADATA
        {
            return Err(invalid(
                "Workbook metadata does not match its retained source.",
            ));
        }
        check_bytes(
            check_bytes(Budget::dataset_bytes(retained, extra_bytes)?, owned_bytes)?,
            metadata.sheets.len() * 128,
        )?;
        let mut names = HashSet::with_capacity(metadata.sheets.len());
        for (index, sheet) in metadata.sheets.iter().enumerate() {
            control.checkpoint(index, None)?;
            if sheet.index as usize != index
                || sheet.name.len() > MAX_SHEET_NAME
                || sheet.kind > 4
                || sheet.visibility > 2
                || sheet.name.trim().is_empty()
                || sheet.name.chars().any(char::is_control)
                || !names.insert(sheet.name.as_str())
            {
                return Err(invalid("Invalid workbook worksheet metadata."));
            }
            if let Some([r0, c0, r1, c1]) = sheet.dimensions {
                if sheet.kind != 0 || r0 > r1 || c0 > c1 || r1 >= 1_048_576 || c1 >= 16384 {
                    return Err(invalid("Invalid worksheet dimensions."));
                }
            }
        }
        Ok(Self {
            snapshot,
            metadata,
            owned_bytes,
        })
    }
}
#[derive(Debug)]
pub struct ExcelPreview {
    workbook: Arc<ExcelWorkbook>,
    selection: ExcelSelection,
    cells: Vec<Cell>,
    columns: Vec<ImportColumnPreview>,
    numeric_source_columns: Vec<bool>,
    samples: Vec<Vec<String>>,
    rows: usize,
    samples_truncated: bool,
    warnings: ExcelWarnings,
    preview_only_bytes: usize,
    owned_bytes: usize,
}
impl ExcelPreview {
    pub fn workbook(&self) -> &Arc<ExcelWorkbook> {
        &self.workbook
    }
    pub fn selection(&self) -> ExcelSelection {
        self.selection
    }
    pub fn columns(&self) -> &[ImportColumnPreview] {
        &self.columns
    }
    pub(crate) fn precision_acknowledgement_required(
        &self,
        index: usize,
        kind: ColumnKind,
    ) -> bool {
        self.numeric_source_columns[index]
            || kind == ColumnKind::Number
                && self.columns[index]
                    .warnings
                    .contains(&ImportWarning::PrecisionRisk)
    }
    pub fn sample_rows(&self) -> &[Vec<String>] {
        &self.samples
    }
    pub fn samples_truncated(&self) -> bool {
        self.samples_truncated
    }
    pub fn row_count(&self) -> usize {
        self.rows
    }
    pub fn warnings(&self) -> &ExcelWarnings {
        &self.warnings
    }
    pub fn preview_only_bytes(&self) -> usize {
        self.preview_only_bytes
    }
    /// Includes the shared workbook; callers retaining both owners count it once.
    pub fn owned_bytes(&self) -> usize {
        self.owned_bytes
    }
    pub fn default_choices(&self) -> AppResult<Vec<ImportColumnChoice>> {
        self.default_choices_with_budget(&[], 0)
    }
    pub fn default_choices_with_budget(
        &self,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Vec<ImportColumnChoice>> {
        let mut live = check_bytes(
            Budget::dataset_bytes(retained, extra_bytes)?,
            self.owned_bytes,
        )?;
        live = check_bytes(live, self.columns.len() * size_of::<ImportColumnChoice>())?;
        for column in &self.columns {
            live = check_bytes(live, column.header.len())?;
        }
        Ok(self
            .columns
            .iter()
            .map(|column| ImportColumnChoice {
                name: column.header.clone(),
                kind: column.inferred_kind,
                missing_rules: MissingRules::None,
                acknowledge_precision_loss: false,
            })
            .collect())
    }
    /// Validates supplied typed records before deriving a preview. No parser runs here.
    pub fn from_cells(
        workbook: Arc<ExcelWorkbook>,
        selection: ExcelSelection,
        cells: Vec<Cell>,
        control: &JobControl,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        control.checkpoint(0, None)?;
        validate_selection(&workbook, selection)?;
        let owned = check_bytes(workbook.owned_bytes(), cell_bytes(&cells, control)?)?;
        let base = check_bytes(Budget::dataset_bytes(retained, extra_bytes)?, owned)?;
        validate_cells(&workbook, selection, &cells, control)?;
        let width = (selection.range[3] - selection.range[1] + 1) as usize;
        let rows =
            (selection.range[2] - selection.range[0] + 1) as usize - usize::from(selection.header);
        // Full column summaries, bounded samples and one temporary cell display string.
        check_bytes(
            base,
            width
                * (size_of::<ImportColumnPreview>()
                    + MAX_STRING
                    + 3 * size_of::<ImportWarning>()
                    + 128
                    + size_of::<bool>())
                + 64 * 1024
                + 2 * MAX_STRING
                + size_of::<Self>(),
        )?;
        let (columns, numeric_source_columns, warnings) =
            column_summaries(selection, &cells, control)?;
        let (samples, samples_truncated) = samples(selection, &cells, rows, width, control)?;
        let mut result = Self {
            workbook,
            selection,
            cells,
            columns,
            numeric_source_columns,
            samples,
            rows,
            samples_truncated,
            warnings,
            preview_only_bytes: 0,
            owned_bytes: 0,
        };
        result.preview_only_bytes = result.storage_bytes(control)?;
        result.owned_bytes = check_bytes(result.workbook.owned_bytes(), result.preview_only_bytes)?;
        check_bytes(
            Budget::dataset_bytes(retained, extra_bytes)?,
            result.owned_bytes,
        )?;
        Ok(result)
    }
    fn storage_bytes(&self, control: &JobControl) -> AppResult<usize> {
        let mut bytes = check_bytes(
            size_of::<Self>() + 2 * size_of::<usize>(),
            cell_bytes(&self.cells, control)?,
        )?;
        bytes = check_bytes(
            bytes,
            self.numeric_source_columns.capacity() * size_of::<bool>(),
        )?;
        bytes = check_bytes(
            bytes,
            self.columns.capacity() * size_of::<ImportColumnPreview>(),
        )?;
        for column in &self.columns {
            bytes = check_bytes(
                bytes,
                column.header.capacity() + column.warnings.capacity() * size_of::<ImportWarning>(),
            )?;
        }
        bytes = check_bytes(bytes, self.samples.capacity() * size_of::<Vec<String>>())?;
        for sample in &self.samples {
            bytes = check_bytes(bytes, sample.capacity() * size_of::<String>())?;
            for text in sample {
                bytes = check_bytes(bytes, text.capacity())?;
            }
        }
        Ok(bytes)
    }
}
fn invalid(message: &'static str) -> AppError {
    AppError::invalid("Excel import", message)
}
fn helper_error(message: String, control: &JobControl) -> AppError {
    if control.check_cancelled().is_err() || message == "cancelled" {
        AppError::Cancelled
    } else if message.contains("budget")
        || message.contains("quota")
        || message.contains("oversized")
        || message.contains("exceeds")
    {
        AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        }
    } else {
        invalid("The workbook worker could not read this selection safely. Check the workbook and try again.")
    }
}
fn worker_config(
    config: &Config,
    selection: ExcelSelection,
    retained_bytes: usize,
    control: &JobControl,
) -> AppResult<Config> {
    if config.probe.is_some() {
        return Err(invalid(
            "Test worker probes are unavailable in Excel import.",
        ));
    }
    check_bytes(
        retained_bytes,
        2 * config.worker.capacity() + 2 * size_of::<Config>() + 2 * size_of::<usize>(),
    )?;
    Ok(Config {
        worker: config.worker.clone(),
        source: Default::default(),
        range: selection.range,
        worksheet_index: selection.worksheet_index,
        quota: config.quota,
        retained_bytes,
        timeout: config.timeout,
        cancel: control.cancellation_flag(),
        probe: None,
    })
}
/// Other retained work excludes this operation's workbook/preview input.
pub fn inspect_with_budget(
    path: &Path,
    config: &Config,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ExcelWorkbook> {
    control.checkpoint(0, None)?;
    let baseline = Budget::dataset_bytes(retained, extra_bytes)?;
    let baseline = check_bytes(
        baseline,
        2 * config.worker.capacity() + 2 * size_of::<Config>() + 2 * size_of::<usize>(),
    )?;
    let snapshot = WorkbookSnapshot::prepare(path, baseline, &control.cancellation_flag())
        .map_err(|e| helper_error(e, control))?;
    check_bytes(
        check_bytes(baseline, snapshot.owned_bytes())?,
        check_bytes(config.quota, 2 * MAX_METADATA + 128 * 1024)?,
    )?;
    let worker = worker_config(
        config,
        ExcelSelection {
            worksheet_index: 0,
            range: [0, 0, 0, 0],
            header: false,
        },
        baseline,
        control,
    )?;
    let metadata = supervisor::inspect(&snapshot, &worker).map_err(|e| helper_error(e, control))?;
    control.checkpoint(1, Some(1))?;
    ExcelWorkbook::from_snapshot(snapshot, metadata, control, retained, extra_bytes)
}
pub fn preview_with_budget(
    workbook: Arc<ExcelWorkbook>,
    selection: ExcelSelection,
    config: &Config,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ExcelPreview> {
    control.checkpoint(0, None)?;
    validate_selection(&workbook, selection)?;
    let base = Budget::dataset_bytes(retained, extra_bytes)?;
    let base = check_bytes(
        base,
        workbook.owned_bytes() - workbook.snapshot.owned_bytes(),
    )?;
    let base = check_bytes(
        base,
        2 * config.worker.capacity() + 2 * size_of::<Config>() + 2 * size_of::<usize>(),
    )?;
    check_bytes(
        check_bytes(base, workbook.snapshot.owned_bytes())?,
        check_bytes(config.quota, 2 * MAX_OUTPUT + 128 * 1024)?,
    )?;
    let worker = worker_config(config, selection, base, control)?;
    let cells = supervisor::run_snapshot(&workbook.snapshot, &worker)
        .map_err(|e| helper_error(e, control))?;
    // run_snapshot has reaped the child before its quota reservation is released.
    control.checkpoint(1, Some(1))?;
    ExcelPreview::from_cells(workbook, selection, cells, control, retained, extra_bytes)
}
fn validate_selection(workbook: &ExcelWorkbook, selection: ExcelSelection) -> AppResult<()> {
    validate_range(selection.range).map_err(|_| AppError::SizeLimit { limit: MAX_CELLS })?;
    let sheet = workbook
        .metadata
        .sheets
        .iter()
        .find(|sheet| sheet.index == selection.worksheet_index)
        .ok_or_else(|| invalid("Select an existing worksheet."))?;
    if sheet.kind != 0 {
        return Err(invalid("Only data worksheets can be imported."));
    }
    Ok(())
}
fn cell_bytes(cells: &Vec<Cell>, control: &JobControl) -> AppResult<usize> {
    let mut bytes = check_bytes(0, cells.capacity() * size_of::<Cell>())?;
    for (index, cell) in cells.iter().enumerate() {
        if index % 128 == 0 {
            control.check_cancelled()?;
        }
        if let Value::Text(text) = &cell.value {
            bytes = check_bytes(bytes, text.capacity())?;
        }
    }
    Ok(bytes)
}
fn validate_cells(
    workbook: &ExcelWorkbook,
    selection: ExcelSelection,
    cells: &[Cell],
    control: &JobControl,
) -> AppResult<()> {
    let [r0, c0, r1, c1] = selection.range;
    let mut previous = None;
    for (index, cell) in cells.iter().enumerate() {
        control.checkpoint(index, Some(cells.len()))?;
        let position = (cell.row, cell.column);
        if cell.row < r0
            || cell.row > r1
            || cell.column < c0
            || cell.column > c1
            || previous.is_some_and(|last| last >= position)
        {
            return Err(invalid("Invalid worksheet cell coordinates or ordering."));
        }
        let valid = match &cell.value {
            Value::Missing => matches!(cell.flags, 0 | 3),
            Value::Number(value) => value.is_finite() && matches!(cell.flags, 32 | 33),
            Value::Text(text) => text.len() <= MAX_STRING && matches!(cell.flags, 0 | 1),
            Value::ExcelDate {
                serial,
                epoch_1904,
                duration,
            } => {
                let flags = u8::from(*epoch_1904) * 8
                    + u8::from(!epoch_1904 && !duration && serial.floor() == 60.0) * 16;
                serial.is_finite()
                    && *epoch_1904 == workbook.metadata.epoch_1904
                    && matches!(cell.flags ^ flags, 0 | 1)
                    && (cell.value.iso_text().is_ok()
                        || !epoch_1904 && !duration && serial.floor() == 60.0)
            }
        };
        if !valid {
            return Err(invalid("Invalid worksheet cell type or provenance flags."));
        }
        previous = Some(position);
    }
    Ok(())
}
fn value_text(value: &Value) -> AppResult<Cow<'_, str>> {
    match value {
        Value::Missing => Ok(Cow::Borrowed("")),
        Value::Text(text) => Ok(Cow::Borrowed(text)),
        Value::Number(value) => Ok(Cow::Owned(value.to_string())),
        Value::ExcelDate {
            serial,
            epoch_1904: false,
            duration: false,
        } if serial.floor() == 60.0 => Ok(Cow::Borrowed("Excel 1900-02-29 (invalid date)")),
        Value::ExcelDate { .. } => value
            .iso_text()
            .map(Cow::Owned)
            .map_err(|_| invalid("The worksheet date cannot be represented as ISO text.")),
    }
}
fn column_summaries(
    selection: ExcelSelection,
    cells: &[Cell],
    control: &JobControl,
) -> AppResult<(Vec<ImportColumnPreview>, Vec<bool>, ExcelWarnings)> {
    let width = (selection.range[3] - selection.range[1] + 1) as usize;
    let mut columns: Vec<_> = (0..width)
        .map(|index| ImportColumnPreview {
            header: format!("Variable {}", index + 1),
            inferred_kind: ColumnKind::Number,
            warnings: Vec::with_capacity(3),
            seen_nonblank: false,
        })
        .collect();
    let mut numeric_source_columns = vec![false; width];
    let mut warnings = ExcelWarnings::default();
    for (index, cell) in cells.iter().enumerate() {
        control.checkpoint(index, Some(cells.len()))?;
        let column = &mut columns[(cell.column - selection.range[1]) as usize];
        if cell.flags & 2 != 0 {
            warnings.missing_formula_cache_cells += 1;
        } else if cell.flags & 1 != 0 {
            warnings.cached_formula_cells += 1;
        }
        if cell.flags & 16 != 0 {
            warnings.fictitious_date_cells += 1;
        }
        if cell.flags & 32 != 0 {
            numeric_source_columns[(cell.column - selection.range[1]) as usize] = true;
            if !column.warnings.contains(&ImportWarning::PrecisionRisk) {
                column.warnings.push(ImportWarning::PrecisionRisk);
            }
        }
        if selection.header && cell.row == selection.range[0] {
            column.header = value_text(&cell.value)?.into_owned();
        } else if !matches!(cell.value, Value::Missing) {
            if let Value::Text(text) = &cell.value {
                if super::preview::precision_risk(text, crate::data::parse_number(text).is_err())
                    && !column.warnings.contains(&ImportWarning::PrecisionRisk)
                {
                    column.warnings.push(ImportWarning::PrecisionRisk);
                }
            }
            column.seen_nonblank = true;
            if !matches!(cell.value, Value::Number(_)) {
                column.inferred_kind = ColumnKind::Text;
            }
        }
    }
    let mut names = HashSet::with_capacity(width);
    for column in &mut columns {
        if !column.seen_nonblank {
            column.inferred_kind = ColumnKind::Text;
        }
        if column.header.trim().is_empty() || column.header.chars().any(char::is_control) {
            column.warnings.push(ImportWarning::InvalidHeader);
        }
        if !names.insert(column.header.as_str()) {
            column.warnings.push(ImportWarning::DuplicateHeader);
        }
    }
    Ok((columns, numeric_source_columns, warnings))
}
fn samples(
    selection: ExcelSelection,
    cells: &[Cell],
    rows: usize,
    width: usize,
    control: &JobControl,
) -> AppResult<(Vec<Vec<String>>, bool)> {
    let mut samples = Vec::with_capacity(rows.min(20));
    let mut bytes = samples.capacity() * size_of::<Vec<String>>();
    let first = selection.range[0] + u32::from(selection.header);
    let mut index = cells.partition_point(|cell| cell.row < first);
    let mut truncated = rows > 20;
    for row in first..first + rows.min(20) as u32 {
        control.checkpoint(row as usize, None)?;
        if bytes + width * size_of::<String>() > 64 * 1024 {
            truncated = true;
            break;
        }
        let mut sample = Vec::with_capacity(width);
        bytes += width * size_of::<String>();
        for column in selection.range[1]..=selection.range[3] {
            let text = if cells
                .get(index)
                .is_some_and(|cell| (cell.row, cell.column) == (row, column))
            {
                let text = value_text(&cells[index].value)?;
                index += 1;
                text
            } else {
                Cow::Borrowed("")
            };
            let length = text.chars().take(120).map(char::len_utf8).sum::<usize>();
            if bytes + length > 64 * 1024 {
                truncated = true;
                return Ok((samples, truncated));
            }
            let mut short = String::with_capacity(length);
            short.extend(text.chars().take(120));
            truncated |= short.len() != text.len();
            bytes += short.capacity();
            sample.push(short);
        }
        samples.push(sample);
    }
    Ok((samples, truncated))
}
/// Builds an immutable dataset from the retained typed capture, without rereading a file.
pub fn commit_preview_with_budget(
    preview: &ExcelPreview,
    choices: &[ImportColumnChoice],
    decisions: ExcelDecisions,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<Dataset> {
    control.checkpoint(0, None)?;
    let baseline = check_bytes(
        Budget::dataset_bytes(retained, extra_bytes)?,
        preview.owned_bytes,
    )?;
    validate_decisions(preview, decisions)?;
    let live = validate_choices(preview, choices, baseline)?;
    let live = construction_bytes(preview.rows, choices, live)?;
    // Text descriptors are precharged above; charge their contents before cloning any.
    let mut live = live;
    for (index, cell) in preview.cells.iter().enumerate() {
        if index % 128 == 0 {
            control.check_cancelled()?;
        }
        if cell.row >= first_data_row(preview)
            && choices[(cell.column - preview.selection.range[1]) as usize].kind == ColumnKind::Text
        {
            live = check_bytes(
                live,
                match &cell.value {
                    Value::Text(text) => text.len(),
                    Value::Missing => 0,
                    _ => 512,
                },
            )?;
        }
    }
    let mut values = allocate_values(preview.rows, choices);
    fill_values(preview, choices, &mut values, control)?;
    let dataset = Dataset::new(build_columns(values, choices, control)?)?;
    let mut datasets = Vec::with_capacity(retained.len() + 1);
    datasets.extend_from_slice(retained);
    datasets.push(&dataset);
    Budget::check_datasets(
        &datasets,
        check_bytes(
            extra_bytes,
            check_bytes(preview.owned_bytes, choice_bytes(choices)?)?,
        )?,
    )?;
    control.checkpoint(preview.rows, Some(preview.rows))?;
    Ok(dataset)
}
fn validate_decisions(preview: &ExcelPreview, decisions: ExcelDecisions) -> AppResult<()> {
    if preview.warnings.cached_formula_cells > 0 && !decisions.acknowledge_cached_formulas {
        return Err(invalid(
            "Acknowledge that cached formula values may be stale.",
        ));
    }
    if preview.warnings.missing_formula_cache_cells > 0 && !decisions.allow_missing_formula_caches {
        return Err(invalid(
            "Choose explicitly to import formulas without cached values as missing.",
        ));
    }
    if preview.warnings.fictitious_date_cells > 0 && !decisions.allow_fictitious_dates_as_missing {
        return Err(invalid(
            "Choose explicitly to import the fictitious Excel 1900-02-29 date as missing.",
        ));
    }
    Ok(())
}
fn validate_choices(
    preview: &ExcelPreview,
    choices: &[ImportColumnChoice],
    baseline: usize,
) -> AppResult<usize> {
    if choices.len() != preview.columns.len() {
        return Err(invalid("Choose a type and unique name for every column."));
    }
    let live = check_bytes(
        check_bytes(baseline, choice_bytes(choices)?)?,
        choices.len() * 128,
    )?;
    let mut names = HashSet::with_capacity(choices.len());
    for (index, choice) in choices.iter().enumerate() {
        if choice.name.trim().is_empty()
            || choice.name.chars().any(char::is_control)
            || !names.insert(choice.name.as_str())
        {
            return Err(invalid(
                "Enter unique nonblank names without control characters.",
            ));
        }
        // Text cannot recover a numeric lexeme already rounded by Excel or the parser.
        if preview.precision_acknowledgement_required(index, choice.kind)
            && !choice.acknowledge_precision_loss
        {
            return Err(invalid(
                "Acknowledge numeric precision for this column before importing.",
            ));
        }
        validate_missing(&choice.missing_rules, choice.kind, "Excel column")?;
    }
    Ok(live)
}
fn first_data_row(preview: &ExcelPreview) -> u32 {
    preview.selection.range[0] + u32::from(preview.selection.header)
}
fn fill_values(
    preview: &ExcelPreview,
    choices: &[ImportColumnChoice],
    values: &mut [Values],
    control: &JobControl,
) -> AppResult<()> {
    let first = first_data_row(preview);
    let mut index = preview.cells.partition_point(|cell| cell.row < first);
    for row in first..first + preview.rows as u32 {
        control.checkpoint((row - first) as usize, Some(preview.rows))?;
        for column in preview.selection.range[1]..=preview.selection.range[3] {
            let offset = (column - preview.selection.range[1]) as usize;
            let cell = if preview
                .cells
                .get(index)
                .is_some_and(|cell| (cell.row, cell.column) == (row, column))
            {
                let cell = &preview.cells[index];
                index += 1;
                Some(cell)
            } else {
                None
            };
            let value = cell
                .filter(|cell| !matches!(cell.value, Value::Missing) && cell.flags & 16 == 0)
                .map(|cell| &cell.value);
            match &mut values[offset] {
                Values::Number(output) => output.push(match value {
                    None => None,
                    Some(Value::Number(number)) => Some(*number),
                    Some(Value::Text(text)) if text.trim().is_empty() => None,
                    Some(Value::Text(text)) => Some(import_number(text, choices[offset].acknowledge_precision_loss).map_err(|_| invalid("A selected numeric column contains text that cannot be imported as a finite number."))?),
                    _ => return Err(invalid("Date and duration columns must be imported as ISO text.")),
                }),
                Values::Text(output) => output.push(value.map(value_text).transpose()?.map(Cow::into_owned)),
            }
        }
    }
    Ok(())
}
