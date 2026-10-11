use super::{wire, MAX_PROJECT_BYTES};
use crate::{
    data::{Budget, Cell, Column, ColumnKind, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
    project::Project,
};
use serde::{
    ser::{SerializeSeq, SerializeStruct},
    Serialize, Serializer,
};
use std::{
    io::{self, Write},
    mem::size_of,
};

/// Encodes without other retained work. The output writer owns any buffered output.
pub fn encode(project: &Project, writer: impl Write, control: &JobControl) -> AppResult<()> {
    encode_with_budget(project, writer, control, &[], 0)
}
/// `extra_bytes` must include retained histories, titles and any buffered output.
/// Prefer a file writer; growing in-memory output needs a caller-managed budget.
pub fn encode_with_budget(
    project: &Project,
    writer: impl Write,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<()> {
    control.checkpoint(0, None)?;
    let mut validation = project.dataset().column_count().saturating_mul(512);
    for run in project.runs() {
        validation = validation
            .checked_add(run.analysis.validation_scratch_bytes_controlled(control)?)
            .ok_or(AppError::SizeLimit {
                limit: crate::data::MAX_OWNED_BYTES,
            })?;
    }
    let extra = extra_bytes
        .checked_add(project.owned_bytes_controlled(control)?)
        .and_then(|n| n.checked_add(validation + size_of::<Project>() + 4096))
        .ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?;
    // Count shared chunk/metadata ownership once across project and caller revisions.
    let references = retained
        .len()
        .checked_add(1)
        .and_then(|n| n.checked_mul(size_of::<&Dataset>()))
        .ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?;
    let extra = extra.checked_add(references).ok_or(AppError::SizeLimit {
        limit: crate::data::MAX_OWNED_BYTES,
    })?;
    Budget::check_datasets(retained, extra)?;
    let mut datasets = Vec::with_capacity(retained.len() + 1);
    datasets.push(project.dataset());
    datasets.extend_from_slice(retained);
    Budget::check_datasets(&datasets, extra)?;
    project.dataset().validate()?;
    super::validation::runs(project.runs(), control)?;
    for run in project.runs() {
        run.analysis.validate_controlled(control)?;
    }
    let mut writer = LimitedWriter {
        inner: writer,
        bytes: 0,
        control,
        error: None,
        next_checkpoint: 0,
    };
    let result = serde_json::to_writer(&mut writer, &Document(project));
    result.map_err(|error| {
        writer.error.take().unwrap_or_else(|| {
            if error.is_io() {
                AppError::Io(io::Error::other("Could not write project JSON."))
            } else {
                AppError::invalid("project", "Project could not be serialized.")
            }
        })
    })?;
    control.checkpoint(writer.bytes, Some(writer.bytes))?;
    Ok(())
}
struct LimitedWriter<'a, W> {
    inner: W,
    bytes: usize,
    control: &'a JobControl,
    error: Option<AppError>,
    next_checkpoint: usize,
}
impl<W: Write> Write for LimitedWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let checkpoint = if self.bytes >= self.next_checkpoint {
            self.next_checkpoint = self.bytes.saturating_add(8192);
            self.control.checkpoint(self.bytes, None)
        } else {
            Ok(())
        };
        let check = checkpoint.and_then(|()| {
            if self
                .bytes
                .checked_add(bytes.len())
                .is_none_or(|n| n > MAX_PROJECT_BYTES)
            {
                return Err(AppError::SizeLimit {
                    limit: MAX_PROJECT_BYTES,
                });
            }
            Ok(())
        });
        if let Err(error) = check {
            self.error = Some(error);
            return Err(io::Error::other("Project limit or cancellation."));
        }
        match self.inner.write(bytes) {
            Ok(written) => {
                self.bytes += written;
                Ok(written)
            }
            Err(error) => {
                let kind = error.kind();
                self.error = Some(AppError::Io(error));
                Err(io::Error::new(kind, "Could not write project JSON."))
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
struct Document<'a>(&'a Project);
impl Serialize for Document<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Next(#[serde(with = "wire::run_id")] crate::operation::RunId);
        #[derive(Serialize)]
        struct MixedRuns<'a>(
            #[serde(with = "super::wire_v2::runs")]
            &'a [std::sync::Arc<crate::project::AnalysisRun>],
        );
        let mut map = serializer.serialize_struct("Project", 11)?;
        map.serialize_field("format", "tessera-project")?;
        map.serialize_field("version", &3_u64)?;
        map.serialize_field("title", self.0.title())?;
        map.serialize_field("dataset", &Data(self.0.dataset()))?;
        map.serialize_field("runs", &MixedRuns(self.0.runs()))?;
        map.serialize_field("next_run_id", &Next(self.0.next_run_id()))?;
        #[derive(Serialize)]
        struct NextColumn(#[serde(with = "wire::column_id")] crate::data::ColumnId);
        map.serialize_field("next_column_id", &NextColumn(self.0.next_column_id()))?;
        map.serialize_field("preparation", self.0.steps())?;
        map.serialize_field("provenance", self.0.provenance())?;
        map.serialize_field("figures", self.0.figures())?;
        map.serialize_field("figure_presets", self.0.presets())?;
        map.end()
    }
}
struct Data<'a>(&'a Dataset);
impl Serialize for Data<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Revision(#[serde(with = "wire::revision_id")] crate::data::RevisionId);
        struct Columns<'a>(&'a [Column]);
        impl Serialize for Columns<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_seq(self.0.iter().map(SavedColumn))
            }
        }
        let mut map = serializer.serialize_struct("Dataset", 3)?;
        map.serialize_field("revision", &Revision(self.0.revision()))?;
        map.serialize_field("rows", &self.0.row_count())?;
        map.serialize_field("columns", &Columns(self.0.columns()))?;
        map.end()
    }
}
struct SavedColumn<'a>(&'a Column);
impl Serialize for SavedColumn<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Id(#[serde(with = "wire::column_id")] crate::data::ColumnId);
        #[derive(Serialize)]
        struct Kind(#[serde(with = "wire::Kind")] ColumnKind);
        #[derive(Serialize)]
        struct Metadata<'a>(#[serde(with = "wire::Metadata")] &'a crate::data::ColumnMetadata);
        let mut map = serializer.serialize_struct("Column", 4)?;
        map.serialize_field("id", &Id(self.0.id()))?;
        map.serialize_field("kind", &Kind(self.0.kind()))?;
        map.serialize_field("metadata", &Metadata(self.0.metadata()))?;
        map.serialize_field("values", &Values(self.0))?;
        map.end()
    }
}
struct Values<'a>(&'a Column);
impl Serialize for Values<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for row in 0..self.0.len() {
            match self.0.kind() {
                ColumnKind::Text => seq.serialize_element(
                    &self.0.text_value(row).map_err(serde::ser::Error::custom)?,
                )?,
                ColumnKind::Number => match self.0.cell(row).map_err(serde::ser::Error::custom)? {
                    Cell::Number(value) => seq.serialize_element(&value)?,
                    Cell::Missing => seq.serialize_element(&None::<f64>)?,
                    _ => return Err(serde::ser::Error::custom("Column type mismatch.")),
                },
            }
        }
        seq.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_limit_is_checked_before_writing_past_it() {
        let control = JobControl::default();
        let mut writer = LimitedWriter {
            inner: Vec::new(),
            bytes: MAX_PROJECT_BYTES - 2,
            control: &control,
            error: None,
            next_checkpoint: 0,
        };
        assert!(writer.write_all(b"123").is_err());
        assert!(matches!(
            writer.error,
            Some(AppError::SizeLimit {
                limit: MAX_PROJECT_BYTES
            })
        ));
        assert!(writer.inner.is_empty());
    }
    #[test]
    fn reliability_precharge_cancellation_precedes_any_output() {
        use crate::{analysis::reliability::*, data::ColumnId, operation::CHECKPOINT_HOOK};
        use std::{cell::Cell as Counter, rc::Rc};
        let dataset = Dataset::new(vec![
            Column::numeric(ColumnId(1), "First", vec![Some(1.), Some(2.), Some(3.)]).unwrap(),
            Column::numeric(ColumnId(2), "Second", vec![Some(2.), Some(3.), Some(4.)]).unwrap(),
        ])
        .unwrap();
        let request = ReliabilityRequest {
            items: vec![ColumnId(1), ColumnId(2)],
            filter: Default::default(),
            frequency_weight: None,
            reversals: vec![],
        };
        let mut run = reliability_with_budget(
            &dataset,
            &request,
            &JobControl::default(),
            &[],
            size_of::<ReliabilityRequest>() + request.heap_bytes().unwrap(),
        )
        .unwrap();
        run.columns[0].description = "wide captured description ".repeat(400);
        let project = Project::new("Capture", dataset)
            .unwrap()
            .with_run(crate::analysis::CompletedAnalysis::Reliability(run))
            .unwrap();
        let control = JobControl::default();
        let cancel = control.clone();
        let calls = Rc::new(Counter::new(0));
        let observed = Rc::clone(&calls);
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                observed.set(observed.get() + 1);
                if observed.get() > 1 {
                    cancel.cancel();
                }
            }))
        });
        let mut output = Vec::new();
        let result = encode(&project, &mut output, &control);
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert!(calls.get() > 1);
        assert!(output.is_empty());
    }
}
