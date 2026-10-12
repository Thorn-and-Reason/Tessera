//! Passive, portable decisions; response data is never included.
use super::{cleaning::CleaningSpec, SurveyChoice, SurveyPresentation, SurveyRole};
use crate::{
    data::{Budget, ColumnId, ColumnKind, Dataset, MAX_OWNED_BYTES},
    error::{AppError, AppResult},
    files::{self, FileStamp, SaveOutcome},
    operation::JobControl,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};
pub const MAX_RECIPE_BYTES: usize = 1024 * 1024;
// Reserve the bounded decoder's worst-case vectors and escaped-string storage.
const WORK_BYTES: usize = MAX_RECIPE_BYTES * 32;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeColumn {
    pub name: String,
    #[serde(with = "kind_wire")]
    pub kind: ColumnKind,
    #[serde(with = "role_wire")]
    pub role: SurveyRole,
    pub cleaning: Option<CleaningSpec>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyRecipe {
    pub name: String,
    pub columns: Vec<RecipeColumn>,
    pub presentation: SurveyPresentation,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    recipe: SurveyRecipe,
}
#[derive(Serialize)]
struct BorrowedEnvelope<'a> {
    version: u32,
    recipe: &'a SurveyRecipe,
}
#[derive(Debug)]
pub struct BoundRecipe {
    pub choices: Vec<SurveyChoice>,
    pub cleanings: Vec<(ColumnId, CleaningSpec)>,
    pub presentation: SurveyPresentation,
}
impl BoundRecipe {
    fn owned_bytes(&self) -> AppResult<usize> {
        let mut bytes = crate::data::AllocationCounter::default();
        bytes.array::<SurveyChoice>(self.choices.capacity())?;
        bytes.array::<(ColumnId, CleaningSpec)>(self.cleanings.capacity())?;
        for (_, spec) in &self.cleanings {
            bytes.add(spec.owned_bytes()?)?;
        }
        Ok(bytes.bytes)
    }
}
fn invalid() -> AppError {
    AppError::invalid(
        "survey recipe",
        "The recipe is invalid or does not match this spreadsheet. Review its columns again.",
    )
}
fn admission(retained: &[&Dataset], extra: usize) -> AppResult<()> {
    Budget::check_datasets(
        retained,
        extra.checked_add(WORK_BYTES).ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?,
    )
}
impl SurveyRecipe {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut bytes = crate::data::AllocationCounter::default();
        bytes.add(std::mem::size_of::<Self>())?;
        bytes.add(self.name.capacity())?;
        bytes.array::<RecipeColumn>(self.columns.capacity())?;
        for column in &self.columns {
            bytes.add(column.name.capacity())?;
            if let Some(spec) = &column.cleaning {
                bytes.add(spec.owned_bytes()?)?;
            }
        }
        Ok(bytes.bytes)
    }
    pub fn validate(&self, control: &JobControl) -> AppResult<()> {
        control.check_cancelled()?;
        if self.name.trim().is_empty()
            || self.name.len() > 4096
            || self.columns.is_empty()
            || self.columns.len() > 10000
        {
            return Err(invalid());
        }
        let mut names = std::collections::HashSet::new();
        let mut included = false;
        for (i, c) in self.columns.iter().enumerate() {
            control.checkpoint(i, Some(self.columns.len()))?;
            if c.name.trim().is_empty()
                || c.name.len() > 16384
                || !names.insert(c.name.as_str())
                || c.role == SurveyRole::Unresolved
                || c.role == SurveyRole::NumericMeasure && c.kind != ColumnKind::Number
            {
                return Err(invalid());
            }
            included |= c.role != SurveyRole::Exclude;
            if let Some(spec) = &c.cleaning {
                spec.validate()?;
                if spec.trim_whitespace && c.kind != ColumnKind::Text
                    || spec.rules.iter().any(|rule| {
                        !mapping_matches(c.kind, &rule.from, false)
                            || !mapping_matches(c.kind, &rule.to, true)
                    })
                {
                    return Err(invalid());
                }
            }
        }
        if !included {
            return Err(invalid());
        }
        if self.owned_bytes()? > MAX_RECIPE_BYTES * 8 {
            return Err(AppError::SizeLimit {
                limit: MAX_RECIPE_BYTES,
            });
        }
        Ok(())
    }
    pub fn capture(
        dataset: &Dataset,
        choices: &[SurveyChoice],
        cleanings: &[(ColumnId, CleaningSpec)],
        presentation: SurveyPresentation,
        name: &str,
        control: &JobControl,
        retained: &[&Dataset],
        extra: usize,
    ) -> AppResult<Self> {
        let datasets: Vec<_> = std::iter::once(dataset)
            .chain(retained.iter().copied())
            .collect();
        admission(&datasets, extra)?;
        control.check_cancelled()?;
        if choices.len() != dataset.columns().len() || choices.len() > 10000 || name.len() > 4096 {
            return Err(invalid());
        }
        let mut cleaning_bytes = 0usize;
        for (_, spec) in cleanings {
            control.check_cancelled()?;
            spec.validate()?;
            cleaning_bytes = cleaning_bytes
                .checked_add(spec.owned_bytes()?)
                .ok_or_else(invalid)?;
            if cleaning_bytes > MAX_RECIPE_BYTES * 8 {
                return Err(AppError::SizeLimit {
                    limit: MAX_RECIPE_BYTES,
                });
            }
        }
        for column in dataset.columns() {
            control.check_cancelled()?;
            cleaning_bytes = cleaning_bytes
                .checked_add(column.name().len())
                .ok_or_else(invalid)?;
            if cleaning_bytes > MAX_RECIPE_BYTES * 8 {
                return Err(AppError::SizeLimit {
                    limit: MAX_RECIPE_BYTES,
                });
            }
        }
        let mut columns = Vec::with_capacity(choices.len());
        for (i, column) in dataset.columns().iter().enumerate() {
            control.checkpoint(i, Some(choices.len()))?;
            if column.name().len() > 16384 {
                return Err(invalid());
            }
            let matched: Vec<_> = choices.iter().filter(|c| c.column == column.id()).collect();
            let rules: Vec<_> = cleanings
                .iter()
                .filter(|(id, _)| *id == column.id())
                .collect();
            if matched.len() != 1 || rules.len() > 1 {
                return Err(invalid());
            }
            columns.push(RecipeColumn {
                name: column.name().into(),
                kind: column.kind(),
                role: matched[0].role,
                cleaning: rules.first().map(|(_, s)| s.clone()),
            });
        }
        if cleanings
            .iter()
            .any(|(id, _)| !dataset.columns().iter().any(|c| c.id() == *id))
        {
            return Err(invalid());
        }
        let recipe = Self {
            name: name.into(),
            columns,
            presentation,
        };
        recipe.validate(control)?;
        // The same bounded serializer proves that capture is portable before returning it.
        encode_with_budget(&recipe, control, retained, extra)?;
        Ok(recipe)
    }
    pub fn bind(&self, dataset: &Dataset, control: &JobControl) -> AppResult<BoundRecipe> {
        admission(&[dataset], 0)?;
        self.validate(control)?;
        if self.columns.len() != dataset.columns().len() {
            return Err(invalid());
        }
        let mut choices = Vec::with_capacity(self.columns.len());
        let mut cleanings = Vec::new();
        for (i, (recipe, column)) in self.columns.iter().zip(dataset.columns()).enumerate() {
            control.checkpoint(i, Some(self.columns.len()))?;
            if recipe.name != column.name() || recipe.kind != column.kind() {
                return Err(invalid());
            }
            choices.push(SurveyChoice {
                column: column.id(),
                role: recipe.role,
            });
            if let Some(spec) = &recipe.cleaning {
                cleanings.push((column.id(), spec.clone()));
            }
        }
        Ok(BoundRecipe {
            choices,
            cleanings,
            presentation: self.presentation,
        })
    }
}
fn mapping_matches(kind: ColumnKind, cell: &crate::data::Cell, missing: bool) -> bool {
    match cell {
        crate::data::Cell::Missing => missing,
        crate::data::Cell::Number(_) => kind == ColumnKind::Number,
        crate::data::Cell::Text(_) => kind == ColumnKind::Text,
    }
}
struct LimitedWriter {
    bytes: Vec<u8>,
}
impl Write for LimitedWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(b.len()) > MAX_RECIPE_BYTES {
            return Err(std::io::Error::other("Recipe too large"));
        }
        self.bytes.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn encode_with_budget(
    recipe: &SurveyRecipe,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<Vec<u8>> {
    admission(retained, extra)?;
    recipe.validate(control)?;
    let mut writer = LimitedWriter { bytes: Vec::new() };
    serde_json::to_writer(&mut writer, &BorrowedEnvelope { version: 1, recipe })
        .map_err(|_| invalid())?;
    control.check_cancelled()?;
    Ok(writer.bytes)
}
pub fn decode_with_budget(
    bytes: &[u8],
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<SurveyRecipe> {
    control.check_cancelled()?;
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err(AppError::SizeLimit {
            limit: MAX_RECIPE_BYTES,
        });
    }
    admission(retained, extra)?;
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.version != 1 {
        return Err(invalid());
    }
    envelope.recipe.validate(control)?;
    Ok(envelope.recipe)
}
pub fn save_with_budget(
    path: &Path,
    recipe: &SurveyRecipe,
    expected: Option<&FileStamp>,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<SaveOutcome> {
    let bytes = encode_with_budget(recipe, control, retained, extra)?;
    files::publish(path, None, expected, control, |writer| {
        writer.write_all(&bytes)?;
        Ok(())
    })
}
pub fn load_with_budget(
    path: &Path,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<(SurveyRecipe, FileStamp)> {
    admission(retained, extra)?;
    control.check_cancelled()?;
    let parent = files::native::Directory::open(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(".")),
    )?;
    let leaf = path.file_name().ok_or_else(invalid)?;
    let file = parent.read(leaf)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(invalid());
    }
    if metadata.len() > MAX_RECIPE_BYTES as u64 {
        return Err(AppError::SizeLimit {
            limit: MAX_RECIPE_BYTES,
        });
    }
    let before = files::stamp_with_limit(path, control, MAX_RECIPE_BYTES)?;
    if before.size > MAX_RECIPE_BYTES as u64 {
        return Err(AppError::SizeLimit {
            limit: MAX_RECIPE_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(before.size as usize);
    file.take(MAX_RECIPE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let recipe = decode_with_budget(&bytes, control, retained, extra)?;
    let after = files::stamp_with_limit(path, control, MAX_RECIPE_BYTES)?;
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    parent.verify_path(&parent.path)?;
    if before != after || digest != before.digest || bytes.len() as u64 != before.size {
        return Err(AppError::Conflict(
            "The recipe changed while opening. Open it again.".into(),
        ));
    }
    Ok((recipe, after))
}
/// Replays only on the original ordered source structure; derived output never rebinds.
pub fn replay_with_budget(
    project: &crate::project::Project,
    recipe: &SurveyRecipe,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<crate::project::Project> {
    let before_bind = extra
        .checked_add(project.owned_bytes_controlled(control)?)
        .and_then(|n| n.checked_add(recipe.owned_bytes().ok()?))
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    let datasets: Vec<_> = std::iter::once(project.dataset())
        .chain(retained.iter().copied())
        .collect();
    admission(&datasets, before_bind)?;
    let bound = recipe.bind(project.dataset(), control)?;
    let replay_storage = bound
        .owned_bytes()?
        .checked_add(recipe.columns.len() * std::mem::size_of::<SurveyChoice>() * 4)
        .ok_or_else(invalid)?;
    let held = extra
        .checked_add(recipe.owned_bytes()?)
        .and_then(|n| n.checked_add(replay_storage))
        .and_then(|n| n.checked_add(project.owned_bytes().ok()?))
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    let mut candidate: Option<crate::project::Project> = None;
    let mut choices = bound.choices;
    for (source, spec) in bound.cleanings {
        control.check_cancelled()?;
        let current = candidate.as_ref().unwrap_or(project);
        let name = super::cleaning::reviewed_name_with_control(current.dataset(), source, control)?;
        let request = super::cleaning::CleaningRequest {
            revision: current.dataset().revision(),
            source,
            name,
            spec,
        };
        let updated = crate::preparation::prepare_with_budget(
            current,
            &request.step()?,
            control,
            &datasets,
            held,
        )?;
        let derived = updated.dataset().columns().last().ok_or_else(invalid)?.id();
        let role = choices
            .iter()
            .find(|choice| choice.column == source)
            .ok_or_else(invalid)?
            .role;
        choices = choices
            .into_iter()
            .map(|choice| {
                if choice.column == source {
                    SurveyChoice {
                        column: source,
                        role: SurveyRole::Exclude,
                    }
                } else {
                    choice
                }
            })
            .chain(std::iter::once(SurveyChoice {
                column: derived,
                role,
            }))
            .collect();
        candidate = Some(updated);
    }
    let current = candidate.as_ref().unwrap_or(project);
    super::generate_presented_with_budget(
        current,
        &super::SurveyRequest {
            revision: current.dataset().revision(),
            choices,
        },
        bound.presentation,
        control,
        &datasets,
        held,
    )
}
mod kind_wire {
    use super::*;
    pub fn serialize<S: serde::Serializer>(v: &ColumnKind, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match v {
            ColumnKind::Number => "number",
            ColumnKind::Text => "text",
        })
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ColumnKind, D::Error> {
        match String::deserialize(d)?.as_str() {
            "number" => Ok(ColumnKind::Number),
            "text" => Ok(ColumnKind::Text),
            _ => Err(serde::de::Error::custom("Unknown column type")),
        }
    }
}
mod role_wire {
    use super::*;
    pub fn serialize<S: serde::Serializer>(v: &SurveyRole, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match v {
            SurveyRole::Unresolved => "unresolved",
            SurveyRole::Exclude => "exclude",
            SurveyRole::Categories => "categories",
            SurveyRole::NumericMeasure => "numeric_measure",
        })
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<SurveyRole, D::Error> {
        match String::deserialize(d)?.as_str() {
            "exclude" => Ok(SurveyRole::Exclude),
            "categories" => Ok(SurveyRole::Categories),
            "numeric_measure" => Ok(SurveyRole::NumericMeasure),
            _ => Err(serde::de::Error::custom("Unknown reporting role")),
        }
    }
}
