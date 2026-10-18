use super::*;
use crate::looks::{
    LookAsset, LookOrigin,
    builtin::{Builtin, DEFAULT_GRID},
    cache::Cache,
};
use std::{
    collections::HashSet,
    fs::File,
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub(super) enum Source {
    Builtin(Builtin),
    Imported(PathBuf),
    Loaded(Arc<LookAsset>),
}

pub(super) struct Pending {
    pub id: u64,
    pub cancel: Arc<AtomicBool>,
}

fn source_note(builtin: Builtin) -> String {
    let source = builtin.source_product();
    format!(
        "Independent creative interpretation derived from published {source} data.\nNot affiliated with or endorsed by the manufacturer."
    )
}
fn asset_builtin(asset: &LookAsset) -> Option<Builtin> {
    let LookOrigin::Builtin { id, .. } = asset.origin() else {
        return None;
    };
    Builtin::ALL.into_iter().find(|builtin| builtin.id() == id)
}
fn asset_name(asset: &LookAsset) -> &str {
    match asset_builtin(asset) {
        Some(builtin) => builtin.name(),
        None => asset.name(),
    }
}

fn loaded_names(assets: &[Arc<LookAsset>]) -> Vec<String> {
    let mut used: HashSet<String> = Builtin::ALL
        .into_iter()
        .map(|builtin| builtin.name().to_owned())
        .chain(["None".into(), "Load LUT…".into()])
        .collect();
    assets
        .iter()
        .map(|asset| {
            let mut name = asset.name().to_owned();
            let mut suffix = 2;
            while !used.insert(name.clone()) {
                name = format!("{} ({suffix})", asset.name());
                suffix += 1;
            }
            name
        })
        .collect()
}

pub(super) fn resolve(
    source: Source,
    cancel: &AtomicBool,
    renderers: &crate::renderer::RendererCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    submissions: &crate::gpu::SubmissionGate,
    cache: Option<&Cache>,
) -> Result<Option<Arc<LookAsset>>, Error> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let asset = match source {
        Source::Loaded(asset) => asset.as_ref().clone(),
        Source::Builtin(builtin) => {
            let cached = cache.and_then(|cache| match cache.load(builtin, DEFAULT_GRID) {
                Ok(asset) => asset,
                Err(e) => {
                    eprintln!("look cache ignored: {e}");
                    None
                }
            });
            match cached {
                Some(asset) => asset,
                None => {
                    let renderer = renderers.get(device, queue)?;
                    let asset = match renderer.bake_builtin(
                        device,
                        queue,
                        submissions,
                        builtin,
                        DEFAULT_GRID,
                        cancel,
                    ) {
                        Ok(asset) => asset,
                        Err(_) if cancel.load(Ordering::Relaxed) => return Ok(None),
                        Err(e) => return Err(e),
                    };
                    if let Some(cache) = cache
                        && let Err(e) = cache.store(builtin, &asset)
                    {
                        eprintln!("look cache write failed: {e}");
                    }
                    asset
                }
            }
        }
        Source::Imported(path) => {
            let file = File::open(&path).map_err(crate::looks::failure)?;
            let mut bytes = Vec::new();
            file.take(crate::looks::MAX_IMPORT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(crate::looks::failure)?;
            if bytes.len() > crate::looks::MAX_IMPORT_BYTES {
                return Err(crate::looks::failure("LUT file exceeds the import budget"));
            }
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let text = std::str::from_utf8(&bytes).map_err(crate::looks::failure)?;
            let name = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("Imported look");
            LookAsset::import_cube(text, name)?
        }
    };
    if cancel.load(Ordering::Relaxed) {
        Ok(None)
    } else {
        crate::gpu::look::Lut::require_limits(&device.limits(), asset.lut())?;
        Ok(Some(Arc::new(asset)))
    }
}

impl Editor {
    pub(super) fn remember_current_look(&mut self) {
        let Some(asset) = self
            .document
            .project
            .output
            .look
            .as_ref()
            .filter(|asset| matches!(asset.origin(), LookOrigin::Imported))
        else {
            return;
        };
        if let Some(existing) = self
            .loaded_looks
            .iter_mut()
            .find(|entry| entry.lut().id() == asset.lut().id())
        {
            *existing = asset.clone();
        } else {
            self.loaded_looks.push(asset.clone());
        }
    }

    pub(super) fn look_render_failure(&self) -> Option<Error> {
        if self.source
            || self.pick.is_some()
            || self.flat_edit.is_some()
            || self.pick_return.is_some()
            || self.analysis_review.is_some()
        {
            return None;
        }
        let id = self.document.project.output.look.as_ref()?.lut().id();
        if let Some(view) = self
            .working
            .displayed
            .as_ref()
            .filter(|view| view.processing.look_id() == Some(id))
        {
            return view.image.look_failure(id);
        }
        self.selected_view()?
            .1
            .image(self.target)
            .ok()?
            .look_failure(id)
    }

    pub(super) fn begin_look(&mut self, source: Source) {
        if self.busy && self.look_pending.is_none() {
            return;
        }
        if let Some(old) = self.look_pending.take() {
            old.cancel.store(true, Ordering::Relaxed);
        }
        let id = self.request_id();
        let cancel = Arc::new(AtomicBool::new(false));
        self.look_pending = Some(Pending {
            id,
            cancel: cancel.clone(),
        });
        self.busy = true;
        self.tasks.begin(Task::Look(id));
        if self
            .commands
            .send(Command::Look(id, source, cancel))
            .is_err()
        {
            self.look_pending = None;
            self.busy = false;
            self.tasks.finish(TaskRequest::Look(id));
            self.report_failure(FailureKind::Look, "Look worker stopped".into());
        }
    }

    pub(super) fn cancel_look(&mut self) {
        if let Some(pending) = &self.look_pending {
            pending.cancel.store(true, Ordering::Relaxed);
            self.tasks.cancelling(TaskRequest::Look(pending.id));
        }
    }

    pub(super) fn retire_look(&mut self) {
        if let Some(pending) = self.look_pending.take() {
            pending.cancel.store(true, Ordering::Relaxed);
            self.tasks.finish(TaskRequest::Look(pending.id));
            self.busy = false;
        }
    }

    pub(super) fn clear_look(&mut self) {
        self.retire_look();
        if self.document.project.output.look.is_some() {
            let before = self.history_snapshot();
            self.document.project.output.look = None;
            self.original.invalidate();
            self.record_edit(before, "Creative look", None);
        }
        self.resolve_failure(FailureKind::Look);
        self.resolve_failure(FailureKind::LookRendering);
    }

    pub(super) fn receive_look(&mut self, id: u64, result: Result<Option<Arc<LookAsset>>, String>) {
        if self.look_pending.as_ref().is_none_or(|p| p.id != id) {
            return;
        }
        let pending = self.look_pending.take().unwrap();
        self.tasks.finish(TaskRequest::Look(id));
        self.busy = false;
        if pending.cancel.load(Ordering::Relaxed) {
            self.status = "Look preparation cancelled".into();
            return;
        }
        match result {
            Ok(Some(asset)) => {
                let before = self.history_snapshot();
                self.document.project.output.look = Some(asset);
                self.remember_current_look();
                self.original.invalidate();
                self.record_edit(before, "Creative look", None);
                self.resolve_failure(FailureKind::Look);
                self.resolve_failure(FailureKind::LookRendering);
            }
            Ok(None) => self.status = "Look preparation cancelled".into(),
            Err(message) => self.report_failure(FailureKind::Look, message),
        }
    }

    pub(super) fn look_controls(&mut self, ui: &mut egui::Ui) {
        let allowed = (!self.busy || self.look_pending.is_some())
            && self.crop_edit.is_none()
            && self.auto_crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.analysis_review.is_none()
            && self.mask_edit.is_none();
        let loaded_names = loaded_names(&self.loaded_looks);
        let name = self.document.project.output.look.as_ref().map_or_else(
            || "None".into(),
            |asset| {
                if matches!(asset.origin(), LookOrigin::Imported)
                    && let Some(index) = self
                        .loaded_looks
                        .iter()
                        .position(|entry| entry.lut().id() == asset.lut().id())
                {
                    loaded_names[index].clone()
                } else {
                    asset_name(asset).to_owned()
                }
            },
        );
        let source = self
            .document
            .project
            .output
            .look
            .as_deref()
            .and_then(asset_builtin);
        let changed = self.document.project.output.look.is_some();
        let mut choice = None;
        let mut clear = false;
        let mut import = false;
        ui.add_enabled_ui(allowed, |ui| {
            let (_, reset) = presentation::section(
                ui,
                "rendering-look",
                "Look",
                presentation::SectionHeader {
                    summary: &name,
                    changed,
                    reset: Some(changed),
                    default_open: Some(true),
                },
                |ui| {
                    presentation::labelled_row(ui, "Look", None, |ui, label| {
                        let response = egui::ComboBox::from_id_salt("creative-look")
                            .width(160.)
                            .truncate()
                            .selected_text(&name)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(!changed, "None").clicked() {
                                    clear = true;
                                }
                                for builtin in Builtin::ALL {
                                    let selected =
                                        self.document.project.output.look.as_ref().is_some_and(
                                            |asset| asset.origin() == &builtin.origin(),
                                        );
                                    if ui
                                        .selectable_label(selected, builtin.name())
                                        .on_hover_text(source_note(builtin))
                                        .clicked()
                                        && !selected
                                    {
                                        choice = Some(Source::Builtin(builtin));
                                    }
                                }
                                if !self.loaded_looks.is_empty() {
                                    ui.separator();
                                    ui.small("Loaded LUTs");
                                    for (asset, name) in self.loaded_looks.iter().zip(&loaded_names)
                                    {
                                        let selected =
                                            self.document.project.output.look.as_ref().is_some_and(
                                                |current| {
                                                    matches!(current.origin(), LookOrigin::Imported)
                                                        && current.lut().id() == asset.lut().id()
                                                },
                                            );
                                        if ui
                                            .push_id(asset.lut().id(), |ui| {
                                                ui.selectable_label(selected, name)
                                            })
                                            .inner
                                            .clicked()
                                            && !selected
                                        {
                                            choice = Some(Source::Loaded(asset.clone()));
                                        }
                                    }
                                }
                                ui.separator();
                                ui.small("ACEScct / AP1 (D60) input and output");
                                ui.small("No output-rendering or colour-conversion LUTs");
                                if ui.selectable_label(false, "Load LUT…").clicked() {
                                    import = true;
                                    ui.close();
                                }
                            });
                        let response = response.response.labelled_by(label);
                        if let Some(builtin) = source {
                            response.on_hover_text(source_note(builtin));
                        }
                    });
                },
            );
            clear |= reset;
        });
        if clear {
            self.clear_look();
        }
        if import
            && let Some(path) = crate::platform::menu::with_native_dialog(|| {
                rfd::FileDialog::new()
                    .set_title("Load scene-referred ACEScct / AP1 LUT")
                    .add_filter("3D look LUT", &["cube"])
                    .pick_file()
            })
        {
            choice = Some(Source::Imported(path));
        }
        if let Some(source) = choice {
            self.begin_look(source);
        }
    }
}

#[cfg(test)]
#[path = "editor_looks_tests.rs"]
mod tests;
