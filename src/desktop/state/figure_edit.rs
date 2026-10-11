use super::*;
use crate::figures::{FigureKey, FigureSettings};

impl DesktopState {
    pub(crate) fn apply_figure_project(&mut self, project: Project) -> AppResult<()> {
        self.idle()?;
        Budget::check_datasets(&[project.dataset()], project.owned_bytes()? + self.extra())?;
        self.project = Arc::new(project);
        self.saved = None;
        self.status = "Selected figure styles applied. Save the project to retain them.".into();
        Ok(())
    }
    pub fn apply_figure_settings(
        &mut self,
        key: &FigureKey,
        settings: FigureSettings,
    ) -> AppResult<()> {
        self.idle()?;
        let project = self
            .project
            .with_figure_with_budget(key, settings, &[], self.extra())?;
        self.project = Arc::new(project);
        self.saved = None;
        self.status = "Figure settings applied. Save the project to retain them.".into();
        Ok(())
    }
    pub(crate) fn remove_figure_settings(&mut self, key: &FigureKey) -> AppResult<()> {
        self.idle()?;
        let project = self
            .project
            .without_figure_with_budget(key, &[], self.extra())?;
        self.apply_figure_project(project)
    }
    pub(crate) fn save_figure_preset(
        &mut self,
        name: &str,
        settings: FigureSettings,
    ) -> AppResult<()> {
        self.idle()?;
        let project =
            self.project
                .with_figure_preset_with_budget(name, settings, &[], self.extra())?;
        Budget::check_datasets(&[project.dataset()], project.owned_bytes()? + self.extra())?;
        self.project = Arc::new(project);
        self.saved = None;
        self.status = "Figure style saved in this project.".into();
        Ok(())
    }
    pub(crate) fn duplicate_report_figure(&mut self, key: &FigureKey) -> AppResult<()> {
        self.idle()?;
        let project = self
            .project
            .duplicate_figure_with_budget(key, &[], self.extra())?;
        Budget::check_datasets(&[project.dataset()], project.owned_bytes()? + self.extra())?;
        self.selected_figure = project.figures().last().map(|f| f.key.clone());
        self.project = Arc::new(project);
        self.saved = None;
        self.status = "Figure presentation duplicated; the captured analysis is shared.".into();
        Ok(())
    }
}
