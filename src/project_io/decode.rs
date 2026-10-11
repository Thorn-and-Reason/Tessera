use super::{
    bounded::{Accounting, Guard},
    wire, MAX_DEPTH, MAX_PROJECT_BYTES,
};
use crate::{
    data::{
        Budget, Cell, Column, ColumnId, ColumnKind, ColumnMetadata, Dataset, RevisionId, MAX_CELLS,
        MAX_OWNED_BYTES,
    },
    error::{AppError, AppResult},
    operation::{JobControl, RunId},
    project::{AnalysisRun, Project},
};
use serde::{de::DeserializeSeed, Deserialize};
use std::{cell::RefCell, io::Read, mem::size_of, sync::Arc};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document<R> {
    #[serde(deserialize_with = "format")]
    format: Format,
    version: Version,
    title: String,
    dataset: Data,
    runs: R,
    #[serde(with = "wire::run_id")]
    next_run_id: RunId,
    #[serde(with = "wire::column_id")]
    next_column_id: ColumnId,
    preparation: Vec<Arc<crate::project::RecordedStep>>,
    provenance: Vec<String>,
    #[serde(default)]
    figures: Vec<crate::figures::FigureRecord>,
    #[serde(default)]
    figure_presets: Vec<crate::figures::FigurePreset>,
}
pub(super) struct FormatToken;
type Format = FormatToken;
impl<'de> Deserialize<'de> for FormatToken {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        format(d)
    }
}
fn format<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Format, D::Error> {
    struct Token;
    impl serde::de::Visitor<'_> for Token {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("tessera-project")
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<(), E> {
            if v == "tessera-project" {
                Ok(())
            } else {
                Err(E::custom("Unsupported project format."))
            }
        }
    }
    if d.deserialize_str(Token).is_err() {
        return Err(serde::de::Error::custom("Unsupported project format."));
    }
    Ok(FormatToken)
}
struct Version;
impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let version = u64::deserialize(d)?;
        if version != 3 {
            return Err(serde::de::Error::custom("Unsupported project version."));
        }
        Ok(Self)
    }
}
#[derive(Deserialize)]
struct MixedRuns(#[serde(with = "super::wire_v2::runs")] Vec<Arc<AnalysisRun>>);
impl<R> Document<R> {
    fn with_runs(
        self,
        f: impl FnOnce(R) -> Vec<Arc<AnalysisRun>>,
    ) -> Document<Vec<Arc<AnalysisRun>>> {
        Document {
            format: self.format,
            version: self.version,
            title: self.title,
            dataset: self.dataset,
            runs: f(self.runs),
            next_run_id: self.next_run_id,
            next_column_id: self.next_column_id,
            preparation: self.preparation,
            provenance: self.provenance,
            figures: self.figures,
            figure_presets: self.figure_presets,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    #[serde(with = "wire::revision_id")]
    revision: RevisionId,
    rows: usize,
    columns: Vec<SavedColumn>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedColumn {
    #[serde(with = "wire::column_id")]
    id: ColumnId,
    #[serde(with = "wire::Kind")]
    kind: ColumnKind,
    #[serde(with = "wire::Metadata")]
    metadata: ColumnMetadata,
    values: Vec<SavedCell>,
}
#[derive(Deserialize)]
struct SavedCell(#[serde(with = "wire::cell")] Cell);

/// Decodes with no other retained work. Applications should use `decode_with_budget`.
pub fn decode(reader: impl Read, control: &JobControl) -> AppResult<Project> {
    decode_with_budget(reader, control, &[], 0)
}
/// `extra_bytes` includes retained titles, histories, input and pending work outside `retained`.
pub fn decode_with_budget(
    mut reader: impl Read,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<Project> {
    control.checkpoint(0, None)?;
    let baseline = Budget::dataset_bytes(retained, extra_bytes)?;
    let mut input = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        control.checkpoint(input.len(), None)?;
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let required = input
            .len()
            .checked_add(read)
            .filter(|n| *n <= MAX_PROJECT_BYTES)
            .ok_or(AppError::SizeLimit {
                limit: MAX_PROJECT_BYTES,
            })?;
        if required > input.capacity() {
            let capacity = required.max(input.capacity().saturating_mul(2));
            // Existing and replacement input allocations may coexist during growth.
            check_owned(baseline, capacity.saturating_add(input.capacity()))?;
            input.reserve_exact(capacity - input.len());
            check_owned(baseline, input.capacity())?;
        }
        input.extend_from_slice(&buffer[..read]);
    }
    // Slice decoding borrows plain strings. Reserve old/new scratch overlap for
    // escaped strings, long numbers and the bounded ignored-value nesting stack
    // before either JSON pass can allocate them.
    let scratch = check_input(&input)?.max(3 * MAX_DEPTH);
    let start = baseline
        .checked_add(input.capacity())
        .and_then(|n| n.checked_add(scratch))
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    check_owned(0, start)?;
    let state = RefCell::new(Accounting {
        bytes: start,
        control,
        error: None,
        steps: 0,
    });
    let mut preflight = serde_json::Deserializer::from_slice(&input);
    control.checkpoint(input.len(), None)?;
    super::dimensions::Dimensions
        .deserialize(Guard {
            inner: &mut preflight,
            state: &state,
            depth: 0,
        })
        .map_err(|error| {
            state
                .borrow_mut()
                .error
                .take()
                .unwrap_or_else(|| json_error(error))
        })?;
    preflight.end().map_err(json_error)?;
    drop(preflight);
    // Dimensions retains only scalar counts. Its visitor work and parser scratch
    // have been dropped; the following typed pass starts from the same baseline.
    state.borrow_mut().bytes = start;
    control.checkpoint(input.len(), None)?;
    let mut json = serde_json::Deserializer::from_slice(&input);
    let guard = Guard {
        inner: &mut json,
        state: &state,
        depth: 0,
    };
    let decoded = Document::<MixedRuns>::deserialize(guard).map(|d| d.with_runs(|r| r.0));
    let document = decoded.map_err(|error| {
        state
            .borrow_mut()
            .error
            .take()
            .unwrap_or_else(|| json_error(error))
    })?;
    json.end().map_err(json_error)?;
    drop(json);
    let released = input.capacity() + scratch;
    drop(input);
    state.borrow_mut().release(released);
    convert(document, &state)
}
fn check_owned(baseline: usize, added: usize) -> AppResult<()> {
    if baseline
        .checked_add(added)
        .is_none_or(|n| n > MAX_OWNED_BYTES)
    {
        return Err(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        });
    }
    Ok(())
}
pub(super) fn check_input(input: &[u8]) -> AppResult<usize> {
    let (mut depth, mut string, mut escaped) = (0_usize, false, false);
    let (mut length, mut longest, mut has_escape) = (0_usize, 0_usize, false);
    let mut unicode_digits = 0;
    let (mut number_length, mut longest_number) = (0_usize, 0_usize);
    for byte in input {
        if string {
            if unicode_digits > 0 {
                unicode_digits -= 1;
            } else if escaped {
                escaped = false;
                if *byte == b'u' {
                    // At most three UTF-8 bytes per code unit (a surrogate pair
                    // uses four, so six is conservative). Include reserve(4) slack.
                    length += 3;
                    unicode_digits = 4;
                } else {
                    length += 1;
                }
            } else if *byte == b'\\' {
                escaped = true;
                has_escape = true;
            } else if *byte == b'"' {
                string = false;
                if has_escape {
                    longest = longest.max(length + 4);
                }
            } else {
                length += 1;
            }
        } else {
            if byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E') {
                number_length += 1;
            } else {
                longest_number = longest_number.max(number_length);
                number_length = 0;
            }
            match byte {
                b'"' => {
                    string = true;
                    length = 0;
                    has_escape = false;
                }
                b'{' | b'[' => {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return Err(AppError::invalid("project", "Project nesting exceeds 64."));
                    }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    // Malformed unterminated strings can grow parser scratch before rejection too.
    if string && has_escape {
        longest = longest.max(length + 4);
    }
    longest_number = longest_number.max(number_length);
    // serde_json's float_roundtrip parser also stores long mantissas in scratch.
    // Its initial significand/decimal prefix is bounded by 32 extra bytes.
    if longest_number > 19 {
        longest = longest.max(longest_number.saturating_add(32));
    }
    Ok(longest.saturating_mul(3).max(usize::from(longest > 0) * 16))
}
fn convert(
    document: Document<Vec<Arc<AnalysisRun>>>,
    state: &RefCell<Accounting<'_>>,
) -> AppResult<Project> {
    let Document {
        format: _,
        version: _,
        title,
        dataset,
        runs,
        next_run_id,
        next_column_id,
        preparation,
        provenance,
        figures,
        figure_presets,
    } = document;
    if dataset
        .rows
        .checked_mul(dataset.columns.len())
        .is_none_or(|n| n > MAX_CELLS)
        || dataset.columns.len() > MAX_CELLS
        || (dataset.columns.is_empty() && dataset.rows != 0)
    {
        return Err(AppError::SizeLimit { limit: MAX_CELLS });
    }
    state.borrow_mut().add(
        dataset
            .columns
            .len()
            .saturating_mul(size_of::<Column>() + 256),
    )?;
    let mut columns = Vec::with_capacity(dataset.columns.len());
    for saved in dataset.columns {
        state.borrow_mut().control.checkpoint(columns.len(), None)?;
        if saved.values.len() != dataset.rows {
            return Err(AppError::invalid(
                "dataset",
                "Column lengths must match the saved row count.",
            ));
        }
        let SavedColumn {
            id,
            kind,
            metadata,
            values,
        } = saved;
        // Reserve constructor storage and temporary vector before either exists.
        let slot = match kind {
            ColumnKind::Number => size_of::<Option<f64>>(),
            ColumnKind::Text => size_of::<Option<String>>(),
        };
        state.borrow_mut().add(
            values.len().saturating_mul(slot).saturating_mul(2)
                + values.len().min(crate::data::CHUNK_SIZE) * slot
                + values.len().div_ceil(crate::data::CHUNK_SIZE) * 128
                + metadata.name.len()
                + metadata.validation_work()?,
        )?;
        let staging = values.capacity() * size_of::<SavedCell>();
        let temporary =
            (values.len() + values.len().min(crate::data::CHUNK_SIZE)) * slot + metadata.name.len();
        let column = match kind {
            ColumnKind::Number => {
                let mut typed = Vec::with_capacity(values.len());
                for SavedCell(cell) in values {
                    typed.push(match cell {
                        Cell::Missing => None,
                        Cell::Number(value) => Some(value),
                        _ => return Err(wrong_type()),
                    });
                }
                Column::numeric(id, &metadata.name, typed)?
            }
            ColumnKind::Text => {
                let mut typed = Vec::with_capacity(values.len());
                for SavedCell(cell) in values {
                    typed.push(match cell {
                        Cell::Missing => None,
                        Cell::Text(value) => Some(value),
                        _ => return Err(wrong_type()),
                    });
                }
                Column::text(id, &metadata.name, typed)?
            }
        };
        columns.push(column.with_metadata(metadata)?);
        drop(column);
        state.borrow_mut().release(staging + temporary);
    }
    state.borrow_mut().add(columns.len().saturating_mul(512))?;
    let dataset = Dataset::from_revision(columns, dataset.revision)?;
    // Snapshot validation makes no assumptions about the current column kinds or names.
    // Its temporary validation sets have bounded keys and are charged before construction.
    let mut validation = dataset.column_count().saturating_mul(512);
    for run in &runs {
        validation = validation
            .checked_add(
                run.analysis
                    .validation_scratch_bytes_controlled(state.borrow().control)?,
            )
            .ok_or(AppError::SizeLimit {
                limit: MAX_OWNED_BYTES,
            })?;
    }
    state
        .borrow_mut()
        .add(validation + size_of::<Project>() + title.len())?;
    state
        .borrow_mut()
        .add(title.len() + runs.len() * size_of::<Arc<AnalysisRun>>() + size_of::<Project>())?;
    super::validation::runs(&runs, &state.borrow().control)?;
    let project = Project::from_parts_controlled(
        &title,
        dataset,
        runs,
        next_run_id,
        &state.borrow().control,
    )?
    .with_provenance(provenance)?
    .with_saved_figures(figures, figure_presets, state.borrow().control)?
    .with_saved_preparation(preparation, next_column_id, state.borrow().control)?;
    state.borrow_mut().control.checkpoint(1, Some(1))?;
    Ok(project)
}
fn wrong_type() -> AppError {
    AppError::invalid("cell", "Cell values must match their column type.")
}
fn json_error(error: serde_json::Error) -> AppError {
    // Do not echo untrusted cell contents, keys or provenance into a UI error.
    AppError::invalid(
        format!("project, line {}, column {}", error.line(), error.column()),
        "Invalid project JSON or unsupported project field.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scratch_only_reserves_escaped_strings_including_unterminated_input() {
        assert_eq!(check_input(br#"{"plain":"ordinary ASCII"}"#).unwrap(), 0);
        let escaped = br#"{"text":"a\"b"}"#;
        assert_eq!(check_input(escaped).unwrap(), 21);
        assert_eq!(check_input(br#""a\"b"#).unwrap(), 21);
        assert!(check_input(br#""\uD83D\uDE00""#).unwrap() >= 3 * 4);
    }
}
