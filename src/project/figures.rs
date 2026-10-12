//! Saved figure presentations and immutable updates.
use super::*;
impl Project {
    pub fn figures(&self) -> &[crate::figures::FigureRecord] {
        &self.figures
    }
    pub fn presets(&self) -> &[crate::figures::FigurePreset] {
        &self.presets
    }
    pub fn figure_settings(
        &self,
        key: &crate::figures::FigureKey,
    ) -> crate::figures::FigureSettings {
        self.figures
            .iter()
            .find(|f| &f.key == key)
            .map(|f| f.settings.clone())
            .unwrap_or_else(|| match key.kind {
                crate::figures::FigureKind::Likert => crate::figures::FigureSettings {
                    width_mm: 280.,
                    height_mm: 340.,
                    font_size: 10.,
                    ..Default::default()
                },
                crate::figures::FigureKind::GroupMeans => crate::figures::FigureSettings {
                    width_mm: 240.,
                    height_mm: 180.,
                    ..Default::default()
                },
                _ => Default::default(),
            })
    }
    fn validate_figure_key(&self, key: &crate::figures::FigureKey) -> AppResult<()> {
        use crate::figures::FigureKind;
        let available = self
            .runs
            .binary_search_by_key(&crate::operation::RunId(key.run), |r| r.id)
            .ok()
            .map(|i| &self.runs[i]);
        if available.is_some_and(|run| match (&run.analysis, key.kind) {
            (CompletedAnalysis::Histogram(_), FigureKind::Histogram) | (CompletedAnalysis::Scatter(_), FigureKind::Scatter) | (CompletedAnalysis::Boxplot(_), FigureKind::Boxplot | FigureKind::GroupMeans) => true,
            (CompletedAnalysis::Describe(a), FigureKind::Distribution(id)) => a.plots.iter().any(|p|p.column.0==id),
            (CompletedAnalysis::Frequencies(_), FigureKind::Likert) => true,
            (CompletedAnalysis::Frequencies(a), FigureKind::Frequency(id)) => a.tables.iter().any(|p|p.column.0==id),
            (CompletedAnalysis::Regression(a), FigureKind::Residual) => matches!(&a.outcome, crate::analysis::RegressionOutcome::Fitted(f) if !f.residual_diagnostics.residual_points.is_empty()),
            (CompletedAnalysis::Regression(a), FigureKind::Qq) => matches!(&a.outcome, crate::analysis::RegressionOutcome::Fitted(f) if !f.residual_diagnostics.qq_points.is_empty()),
            _ => false,
        })
        {
            Ok(())
        } else {
            Err(AppError::invalid(
                "figure",
                "The figure must belong to a saved result.",
            ))
        }
    }
    pub fn with_figure(
        &self,
        key: &crate::figures::FigureKey,
        settings: crate::figures::FigureSettings,
    ) -> AppResult<Self> {
        self.with_figure_with_budget(key, settings, &[], 0)
    }
    pub fn with_figure_with_budget(
        &self,
        key: &crate::figures::FigureKey,
        settings: crate::figures::FigureSettings,
        retained: &[&Dataset],
        extra: usize,
    ) -> AppResult<Self> {
        if self.figures.len() >= 1024 && !self.figures.iter().any(|f| f.key == *key) {
            return Err(AppError::invalid(
                "figures",
                "A project can retain at most 1024 figure presentations.",
            ));
        }
        self.validate_figure_key(key)?;
        settings.validate()?;
        self.check_update_budget(
            None,
            retained,
            extra,
            self.copied_shell_bytes(self.runs.len())?
                + self.copied_provenance_bytes()
                + size_of::<crate::figures::FigureRecord>()
                + settings.heap_bytes(),
            0,
        )?;
        let mut figures = Vec::with_capacity(self.figures.len() + 1);
        figures.extend(self.figures.iter().filter(|f| &f.key != key).cloned());
        figures.push(crate::figures::FigureRecord {
            key: *key,
            settings,
        });
        let next = Self {
            figures,
            title: self.title.clone(),
            dataset: Arc::clone(&self.dataset),
            runs: self.runs.clone(),
            steps: self.steps.clone(),
            next_column: self.next_column,
            next_run: self.next_run,
            provenance: self.provenance.clone(),
            presets: self.presets.clone(),
        };
        Budget::check_datasets(&[self.dataset()], next.owned_bytes()?)?;
        Ok(next)
    }
    pub fn without_figure_with_budget(
        &self,
        key: &crate::figures::FigureKey,
        retained: &[&Dataset],
        extra: usize,
    ) -> AppResult<Self> {
        self.validate_figure_key(key)?;
        self.check_update_budget(
            None,
            retained,
            extra,
            self.copied_shell_bytes(self.runs.len())? + self.copied_provenance_bytes(),
            0,
        )?;
        let next = Self {
            figures: self
                .figures
                .iter()
                .filter(|f| &f.key != key)
                .cloned()
                .collect(),
            title: self.title.clone(),
            dataset: Arc::clone(&self.dataset),
            runs: self.runs.clone(),
            steps: self.steps.clone(),
            next_column: self.next_column,
            next_run: self.next_run,
            provenance: self.provenance.clone(),
            presets: self.presets.clone(),
        };
        Budget::check_datasets(&[self.dataset()], next.owned_bytes()?)?;
        Ok(next)
    }
    pub fn with_figure_preset(
        &self,
        name: &str,
        settings: crate::figures::FigureSettings,
    ) -> AppResult<Self> {
        self.with_figure_preset_with_budget(name, settings, &[], 0)
    }
    pub fn with_figure_preset_with_budget(
        &self,
        name: &str,
        settings: crate::figures::FigureSettings,
        retained: &[&Dataset],
        extra: usize,
    ) -> AppResult<Self> {
        if self.presets.len() >= 256 && !self.presets.iter().any(|p| p.name == name) {
            return Err(AppError::invalid(
                "presets",
                "A project can retain at most 256 figure presets.",
            ));
        }
        settings.validate()?;
        if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err(AppError::invalid(
                "preset name",
                "Enter a printable name of at most 128 bytes.",
            ));
        }
        self.check_update_budget(
            None,
            retained,
            extra,
            self.copied_shell_bytes(self.runs.len())?
                + self.copied_provenance_bytes()
                + size_of::<crate::figures::FigurePreset>()
                + name.len()
                + settings.heap_bytes(),
            0,
        )?;
        let mut presets = Vec::with_capacity(self.presets.len() + 1);
        presets.extend(self.presets.iter().filter(|p| p.name != name).cloned());
        presets.push(crate::figures::FigurePreset {
            name: name.into(),
            settings,
        });
        let next = Self {
            presets,
            title: self.title.clone(),
            dataset: Arc::clone(&self.dataset),
            runs: self.runs.clone(),
            steps: self.steps.clone(),
            next_column: self.next_column,
            next_run: self.next_run,
            provenance: self.provenance.clone(),
            figures: self.figures.clone(),
        };
        Budget::check_datasets(&[self.dataset()], next.owned_bytes()?)?;
        Ok(next)
    }
    pub fn duplicate_figure(&self, key: &crate::figures::FigureKey) -> AppResult<Self> {
        self.duplicate_figure_with_budget(key, &[], 0)
    }
    pub fn duplicate_figure_with_budget(
        &self,
        key: &crate::figures::FigureKey,
        retained: &[&Dataset],
        extra: usize,
    ) -> AppResult<Self> {
        self.check_update_budget(
            None,
            retained,
            extra,
            self.copied_shell_bytes(self.runs.len())?
                + self.copied_provenance_bytes()
                + size_of::<crate::figures::FigureRecord>()
                + self
                    .figures
                    .iter()
                    .find(|f| f.key == *key)
                    .map_or(0, |f| f.settings.heap_bytes()),
            0,
        )?;
        self.validate_figure_key(key)?;
        let variant = self
            .figures
            .iter()
            .filter(|f| f.key.run == key.run && f.key.kind == key.kind)
            .map(|f| f.key.variant)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| AppError::invalid("figure", "Too many figure variants."))?;
        self.with_figure_with_budget(
            &crate::figures::FigureKey {
                variant,
                ..key.clone()
            },
            self.figure_settings(key),
            retained,
            extra,
        )
    }
    pub(crate) fn with_saved_figures(
        self,
        figures: Vec<crate::figures::FigureRecord>,
        presets: Vec<crate::figures::FigurePreset>,
        control: &crate::operation::JobControl,
    ) -> AppResult<Self> {
        control.check_cancelled()?;
        if figures.len() > 1024 || presets.len() > 256 {
            return Err(AppError::invalid(
                "figures",
                "A project can retain at most 1024 presentations and 256 presets.",
            ));
        }
        for (i, f) in figures.iter().enumerate() {
            control.checkpoint(i, None)?;
            self.validate_figure_key(&f.key)?;
            f.settings.validate()?;
            if figures[..i].iter().any(|p| p.key == f.key) {
                return Err(AppError::invalid("figure", "Duplicate figure key."));
            }
        }
        for (i, p) in presets.iter().enumerate() {
            control.checkpoint(i, None)?;
            p.settings.validate()?;
            if p.name.trim().is_empty()
                || p.name.len() > 128
                || p.name.chars().any(char::is_control)
                || presets[..i].iter().any(|q| q.name == p.name)
            {
                return Err(AppError::invalid(
                    "preset",
                    "Invalid or duplicate preset name.",
                ));
            }
        }
        let next = Self {
            figures,
            presets,
            ..self
        };
        Budget::check_datasets(&[next.dataset()], next.owned_bytes()?)?;
        Ok(next)
    }
    pub(super) fn figure_bytes(&self) -> AppResult<usize> {
        let mut counter = AllocationCounter::default();
        counter.array::<crate::figures::FigureRecord>(self.figures.capacity())?;
        counter.array::<crate::figures::FigurePreset>(self.presets.capacity())?;
        for f in &self.figures {
            counter.add(f.settings.heap_bytes())?;
        }
        for p in &self.presets {
            counter.add(p.name.capacity())?;
            counter.add(p.settings.heap_bytes())?;
        }
        Ok(counter.bytes)
    }
}
