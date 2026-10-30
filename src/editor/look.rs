use super::*;
use crate::looks::{
    LookAsset, LookOrigin,
    builtin::{Builtin, DEFAULT_GRID},
    cache::Cache,
};
use std::{
    fs::File,
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(test)]
mod resolver_tests;
#[cfg(test)]
mod tests;
mod ui;

#[derive(Clone)]
pub(super) enum Source {
    Builtin(Builtin),
    Imported(PathBuf),
    Loaded(Arc<LookAsset>),
}

#[derive(Default)]
pub(super) struct Session {
    pub services: Option<Services>,
    pub loaded: Vec<Arc<LookAsset>>,
}

pub(super) struct Operation {
    pub activity: u64,
    task: Option<async_executor::Task<()>>,
}

/// Shared derived assets and lazy baking resources survive document changes.
struct CacheOwner {
    value: Option<Cache>,
    baker: crate::looks::bake::Baker,
    #[cfg(test)]
    _directory: tempfile::TempDir,
}

#[cfg(test)]
type PreparedObserver = Arc<dyn Fn(bool, crate::cancellation::Cancellation) + Send + Sync>;

#[derive(Clone)]
pub(super) struct Services {
    cache: Arc<CacheOwner>,
    #[cfg(test)]
    prepared: Option<PreparedObserver>,
}

impl Services {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        submissions: crate::gpu::SubmissionGate,
    ) -> Self {
        #[cfg(test)]
        let directory = tempfile::tempdir().expect("look cache fixture directory");
        let value = {
            #[cfg(test)]
            {
                Some(Cache::at(directory.path().into()))
            }
            #[cfg(not(test))]
            {
                Cache::default_location()
            }
        };
        Self {
            cache: Arc::new(CacheOwner {
                value,
                baker: crate::looks::bake::Baker::new(device, queue, submissions),
                #[cfg(test)]
                _directory: directory,
            }),
            #[cfg(test)]
            prepared: None,
        }
    }

    pub async fn prepare(self, source: Source) -> Result<Option<Arc<LookAsset>>, String> {
        crate::background::run(move |cancelled| {
            let result = prepare_asset(
                source,
                &cancelled.flag(),
                &self.cache.baker,
                self.cache.value.as_ref(),
            );
            #[cfg(test)]
            if let Some(observe) = &self.prepared {
                observe(matches!(&result, Ok(Some(_))), cancelled);
            }
            result
        })
        .await
        .map_err(|_| "Look preparation worker panicked".to_owned())?
        .map_err(|error| error.to_string())
    }
}

pub(super) fn prepare_asset(
    source: Source,
    cancel: &AtomicBool,
    baker: &crate::looks::bake::Baker,
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
                    log::warn!(error = e.to_string().as_str(); "look_cache_ignored");
                    None
                }
            });
            match cached {
                Some(asset) => asset,
                None => {
                    let asset = match baker.bake(builtin, DEFAULT_GRID, cancel) {
                        Ok(asset) => asset,
                        Err(_) if cancel.load(Ordering::Relaxed) => return Ok(None),
                        Err(e) => return Err(e),
                    };
                    if let Some(cache) = cache
                        && let Err(e) = cache.store(builtin, &asset)
                    {
                        log::warn!(error = e.to_string().as_str(); "look_cache_write_failed");
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
        crate::gpu::look::Lut::require_limits(&baker.device_limits(), asset.lut())?;
        Ok(Some(Arc::new(asset)))
    }
}

impl Editor {
    pub(super) fn select_look(&mut self, source: Source, cx: &UiContext<'_, Self>) {
        if self.busy && self.document.look.is_none() {
            return;
        }
        self.retire_look_selection();
        let activity = self.request_id();
        self.document.look = Some(Operation {
            activity,
            task: None,
        });
        self.busy = true;
        self.tasks.begin(Task::Look(activity));
        let services = self.look.services.clone();
        let task = cx.spawn(move |cx| async move {
            let result = match services {
                Some(services) => services.prepare(source).await,
                None => Err("Look services unavailable".into()),
            };
            cx.update(|editor| {
                editor.retire_look_selection();
                match result {
                    Ok(Some(asset)) => {
                        let before = editor.history_snapshot();
                        editor.document.state.project.output.look = Some(asset);
                        editor.remember_imported_look();
                        editor.invalidate_original();
                        editor.record_edit(before, "Creative look", None);
                        editor.resolve_failure(FailureKind::Look);
                        editor.resolve_failure(FailureKind::LookRendering);
                    }
                    Ok(None) => editor.status = "Look preparation cancelled".into(),
                    Err(message) => editor.report_failure(FailureKind::Look, message),
                }
            });
        });
        if let Some(task) = task {
            self.document.look.as_mut().unwrap().task = Some(task);
        } else {
            self.retire_look_selection();
            self.report_failure(FailureKind::Look, "Application stopped".into());
        }
    }

    pub(super) fn cancel_look_selection(&mut self) {
        self.retire_look_selection();
        self.status = "Look preparation cancelled".into();
    }

    pub(super) fn retire_look_selection(&mut self) {
        if let Some(pending) = self.document.look.take()
            && self.tasks.finish(TaskRequest::Look(pending.activity))
        {
            self.busy = false;
        }
    }

    pub(super) fn clear_look(&mut self) {
        self.retire_look_selection();
        if self.document.state.project.output.look.is_some() {
            let before = self.history_snapshot();
            self.document.state.project.output.look = None;
            self.invalidate_original();
            self.record_edit(before, "Creative look", None);
        }
        self.resolve_failure(FailureKind::Look);
        self.resolve_failure(FailureKind::LookRendering);
    }

    pub(super) fn remember_imported_look(&mut self) {
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
            .look
            .loaded
            .iter_mut()
            .find(|entry| entry.lut().id() == asset.lut().id())
        {
            *existing = asset.clone();
        } else {
            self.look.loaded.push(asset.clone());
        }
    }
}
