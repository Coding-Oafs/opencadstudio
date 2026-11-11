use super::{Message, OpenCADStudio};
use crate::ui::{
    dock::PanelId,
    window::workspace::{CancelJob, ExplorerRow, JobKind, JobRow, Target},
};
use iced::Task;

impl OpenCADStudio {
    pub(super) fn open_workspace_panel(&mut self, id: PanelId) {
        match id {
            PanelId::ProjectExplorer => self.show_project_explorer = true,
            PanelId::Jobs => self.show_jobs = true,
            _ => return,
        }
        if self.dock.location(id).is_none() {
            self.dock.dock(id, super::config::DockSide::Right, 0);
        }
        self.dock_expanded = Some(id);
        self.ribbon.close_dropdown();
        self.save_config();
    }

    pub(super) fn explorer_rows(&self) -> Vec<ExplorerRow> {
        let mut rows = Vec::new();
        for tab in self.tabs.iter().filter(|t| !t.is_start) {
            rows.push(ExplorerRow {
                group: "Drawings",
                name: tab.tab_title.clone(),
                detail: tab
                    .current_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "Unsaved drawing".into()),
                target: Target::Drawing(tab.id),
            });
        }
        let tab = &self.tabs[self.active_tab];
        #[cfg(not(target_arch = "wasm32"))]
        {
            for cloud in &tab.point_cloud.sources {
                rows.push(ExplorerRow {
                    group: "Point clouds",
                    name: cloud
                        .source_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    detail: format!(
                        "{} source points · {} edit transactions",
                        cloud.sample.metadata.point_count,
                        cloud.edits.transaction_count()
                    ),
                    target: Target::Cloud(tab.id, cloud.id.clone()),
                });
            }
            for layer in &tab.gis_layers {
                rows.push(ExplorerRow {
                    group: "GIS layers",
                    name: layer.name.clone(),
                    detail: format!("{} features · EPSG:{}", layer.features.len(), layer.epsg),
                    target: Target::Gis(tab.id, layer.name.clone()),
                });
            }
            if let Some(terrain) = &tab.terrain {
                rows.push(ExplorerRow {
                    group: "Surfaces",
                    name: terrain.source.clone(),
                    detail: format!(
                        "{} × {} cells · {}",
                        terrain.surface.columns, terrain.surface.rows, terrain.crs_label
                    ),
                    target: Target::Terrain(tab.id),
                });
            }
        }
        for entity in tab.scene.document.entities().take(200) {
            let h = entity.common().handle;
            rows.push(ExplorerRow {
                group: "CAD objects",
                name: format!(
                    "{} · {:X}",
                    crate::entities::names::ui_name(entity),
                    h.value()
                ),
                detail: format!("Layer: {}", entity.common().layer),
                target: Target::Entity(tab.id, h.value()),
            });
        }
        rows
    }

    pub(super) fn focus_workspace_item(&mut self, target: Target) -> Task<Message> {
        let id = match &target {
            Target::Drawing(id)
            | Target::Entity(id, _)
            | Target::Cloud(id, _)
            | Target::Terrain(id)
            | Target::Gis(id, _) => *id,
        };
        let Some(i) = self.tabs.iter().position(|tab| tab.id == id) else {
            return Task::none();
        };
        // Use the normal tab switch path; it restores ribbon and input state.
        let task = if i != self.active_tab {
            self.update(Message::TabSwitch(i))
        } else {
            Task::none()
        };
        if self.active_tab != i {
            return task;
        }
        match target {
            Target::Drawing(_) => {}
            Target::Entity(_, raw) => {
                let handle = acadrust::Handle::new(raw);
                if self.tabs[i].scene.document.get_entity(handle).is_some() {
                    self.tabs[i]
                        .scene
                        .replace_selection([handle].into_iter().collect());
                    self.tabs[i].scene.remember_current_view();
                    self.tabs[i].scene.zoom_to_entities(&[handle]);
                    self.refresh_properties();
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Target::Cloud(_, source) => {
                self.focus_cloud_source(i, &source);
            }
            #[cfg(not(target_arch = "wasm32"))]
            Target::Terrain(_) => {
                if let Some(d) = &self.tabs[i].terrain {
                    let s = &d.surface;
                    let range = d.elevation_range().unwrap_or((0., 0.));
                    let bounds = (
                        [s.origin[0], s.origin[1], range.0],
                        [
                            s.origin[0] + s.cell_size * s.columns.saturating_sub(1) as f64,
                            s.origin[1] + s.cell_size * s.rows.saturating_sub(1) as f64,
                            range.1,
                        ],
                    );
                    self.tabs[i].scene.fit_external_bounds(bounds.0, bounds.1);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Target::Gis(_, name) => {
                self.command_line.push_info(format!("GIS layer: {name}. Use GIS & project tools for topology, reprojection and export.").as_str());
                self.command_line.history_open = true;
            }
            #[cfg(target_arch = "wasm32")]
            _ => {}
        }
        task
    }

    pub(super) fn workspace_jobs(&self) -> Vec<JobRow> {
        let mut rows = Vec::new();
        if let Some(opening) = &self.opening {
            use std::sync::atomic::Ordering;
            rows.push(JobRow {
                name: format!("Open / import {}", opening.name),
                detail: crate::io::open_phase_name(opening.state.phase.load(Ordering::Acquire))
                    .into(),
                completed: opening.state.basis_points.load(Ordering::Relaxed) as u64,
                total: 10000,
                cancel: None,
            });
        }
        if let Some(job) = &self.basemap_job {
            let (done, total, failed) = job.snapshot();
            rows.push(JobRow {
                name: "Basemap tiles".into(),
                detail: format!(
                    "Drawing {} · {failed} failed tiles",
                    self.job_tab_name(job.tab_id)
                ),
                completed: done as u64,
                total: total as u64,
                cancel: Some(CancelJob {
                    tab: job.tab_id,
                    kind: JobKind::Basemap,
                    source: String::new(),
                    token: job.id,
                }),
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(job) = &self.terrain_job {
                let (done, total, failed) = job.snapshot();
                rows.push(JobRow {
                    name: "Terrain download and surface build".into(),
                    detail: format!(
                        "Drawing {} · {failed} failed tiles; grid and snapshot follow download",
                        self.job_tab_name(job.tab_id)
                    ),
                    completed: done as u64,
                    total: total as u64,
                    cancel: Some(CancelJob {
                        tab: job.tab_id,
                        kind: JobKind::Terrain,
                        source: String::new(),
                        token: job.id,
                    }),
                });
            }
            rows.extend(self.point_cloud_job_rows());
            for id in self.active_save_jobs.keys() {
                rows.push(JobRow {
                    name: "Save drawing".into(),
                    detail: self.job_tab_name(*id),
                    completed: 0,
                    total: 0,
                    cancel: None,
                });
            }
        }
        rows
    }

    fn job_tab_name(&self, id: u64) -> String {
        self.tabs
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.tab_title.clone())
            .unwrap_or_else(|| "closed drawing".into())
    }

    pub(super) fn workspace_job_history(&self) -> Vec<String> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((_, project)) = &self.tabs[self.active_tab].spatial_project {
            return project
                .jobs
                .iter()
                .rev()
                .take(40)
                .map(|job| {
                    format!(
                        "{} · {:?} · {}{}",
                        job.display_name,
                        job.status,
                        job.current_stage,
                        job.error
                            .as_ref()
                            .map(|e| format!(" · {e}"))
                            .unwrap_or_default()
                    )
                })
                .collect();
        }
        Vec::new()
    }

    pub(super) fn cancel_workspace_job(&mut self, key: CancelJob) {
        use std::sync::atomic::Ordering;
        match key.kind {
            JobKind::Basemap => {
                if let Some(job) = self
                    .basemap_job
                    .as_ref()
                    .filter(|j| j.tab_id == key.tab && j.id == key.token)
                {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            JobKind::Terrain => {
                if let Some(job) = self.terrain_job.take() {
                    if job.tab_id == key.tab && job.id == key.token {
                        job.cancel.store(true, Ordering::Relaxed);
                    } else {
                        self.terrain_job = Some(job);
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            _ => self.cancel_cloud_job(&key),
            #[cfg(target_arch = "wasm32")]
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn explorer_focus_uses_document_identity_after_tab_reordering() {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.automation_op(r#"{"op":"run","cmd":"LINE 0,0 10,10"}"#);
        let i = app.active_tab;
        let id = app.tabs[i].id;
        let handle = app.tabs[i]
            .scene
            .document
            .entities()
            .next()
            .unwrap()
            .common()
            .handle;
        app.tabs
            .insert(0, super::super::document::DocumentTab::new_drawing(602));
        app.active_tab = 0;
        let _ = app.focus_workspace_item(Target::Entity(id, handle.value()));
        assert_eq!(app.tabs[app.active_tab].id, id);
        assert!(app.tabs[app.active_tab]
            .scene
            .selected_entities()
            .iter()
            .any(|(h, _)| *h == handle));
        assert_eq!(app.tabs[0].scene.selected_entities().len(), 0);
    }

    #[test]
    fn cancellation_ignores_stale_job_and_document_ids() {
        let mut app = OpenCADStudio::new_for_test();
        let cancelled = Arc::new(AtomicBool::new(false));
        app.basemap_job = Some(super::super::basemap::BasemapJob {
            id: 52,
            tab_id: 27,
            total: 10,
            completed: Arc::new(AtomicUsize::new(2)),
            failed: Arc::new(AtomicUsize::new(0)),
            cancel: cancelled.clone(),
        });
        let mut key = CancelJob {
            tab: 27,
            token: 51,
            kind: JobKind::Basemap,
            source: String::new(),
        };
        app.cancel_workspace_job(key.clone());
        assert!(!cancelled.load(Ordering::Relaxed));
        key.token = 52;
        key.tab = 28;
        app.cancel_workspace_job(key.clone());
        assert!(!cancelled.load(Ordering::Relaxed));
        key.tab = 27;
        app.cancel_workspace_job(key);
        assert!(cancelled.load(Ordering::Relaxed));
    }
}
