use super::*;

#[cfg(test)]
mod workflow_tests;

#[cfg(test)]
pub(super) struct Submission {
    pub job: Job,
    pub reply: futures_channel::oneshot::Sender<Outcome>,
}

#[derive(Clone)]
pub(super) enum Services {
    Production(Arc<production_worker::Worker>),
    #[cfg(test)]
    Controlled(mpsc::Sender<Submission>),
}

#[derive(Debug)]
pub(super) struct Stopped;

impl Services {
    pub async fn process(
        self,
        job: Job,
        sources: Option<Arc<dyn crate::source::SourceProvider>>,
    ) -> Result<Outcome, Stopped> {
        match self {
            Self::Production(worker) => worker.process(job, sources.ok_or(Stopped)?).await,
            #[cfg(test)]
            Self::Controlled(commands) => {
                let (reply, result) = futures_channel::oneshot::channel();
                commands
                    .send(Submission { job, reply })
                    .map_err(|_| Stopped)?;
                result.await.map_err(|_| Stopped)
            }
        }
    }
}

/// Document-owned continuations include CPU/disk work whose GPU ticket has retired.
#[derive(Default)]
pub(in crate::editor) struct State {
    tasks: Vec<Pending>,
}

struct Pending {
    ticket: u64,
    generation: Arc<std::sync::atomic::AtomicU64>,
    _task: async_executor::Task<()>,
    _retire: crate::cancellation::CancelOnDrop,
}

impl State {
    #[cfg(test)]
    pub(in crate::editor) fn pending(&self) -> bool {
        !self.tasks.is_empty()
    }
}

/// Visible intent stays lightweight; source/recipe ownership begins with each captured job.
#[derive(Default)]
pub(super) enum Demand {
    #[default]
    None,
    Visible(Vec<SourceId>),
    Hidden,
}

impl Editor {
    pub(super) fn request_thumbnail(
        &mut self,
        key: ImageKey,
        path: PathBuf,
        source: Source,
        render: bool,
        priority: source_loading::Priority,
        cx: &UiContext<'_, Self>,
    ) {
        if self.document.thumbnails.pending.is_some()
            || self.document.thumbnails.cache.get(&key).is_some()
        {
            return;
        }
        let Some(services) = self.document.thumbnails.services.clone() else {
            if self.document.thumbnails.gpu.is_some() {
                self.document
                    .thumbnails
                    .cache
                    .insert(key, Err("Thumbnail worker stopped".into()));
            }
            return;
        };
        let generation = self.document.thumbnails.generation.clone();
        let ticket = generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let previous = render
            .then(|| {
                self.document
                    .thumbnails
                    .compatible_product(&key)
                    .map(|(_, product)| product.clone())
            })
            .flatten();
        let cancelled = crate::cancellation::Cancellation::default();
        let retire = cancelled.cancel_on_drop();
        let job = Job {
            generation: generation.clone(),
            cancelled,
            reference: self
                .document
                .exposure_slot(&self.document.exposures, source.id),
            ticket,
            key: key.clone(),
            path,
            source,
            correction: self.document.correction.clone(),
            render,
            priority,
            previous,
        };
        self.document.thumbnails.pending_render = render;
        self.document.thumbnails.pending = Some((ticket, key.clone()));
        let fallback_key = key.clone();

        let sources = self.document.thumbnails.acquisition();
        let identity = generation.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let result = services.process(job, sources).await;
            cx.update(|editor| {
                let Some(index) = editor
                    .document
                    .thumbnail_work
                    .tasks
                    .iter()
                    .position(|pending| {
                        pending.ticket == ticket && Arc::ptr_eq(&pending.generation, &identity)
                    })
                else {
                    return;
                };
                let pending = editor.document.thumbnail_work.tasks.swap_remove(index);
                if Arc::ptr_eq(&identity, &editor.document.thumbnails.generation) {
                    editor
                        .document
                        .state
                        .capture_exposure(&editor.document.exposures);
                    match result {
                        Ok(mut outcome) => {
                            if let Some(proxy) = outcome.proxy.take()
                                && let Some(key) = outcome.key.source.store_key()
                                && let Err(error) = editor.document.cache.publish_proxy(key, proxy)
                            {
                                log::warn!(error = error.to_string().as_str(); "thumbnail_proxy_publication_failed");
                            }
                            editor.document.thumbnails.complete(outcome, &editor.document.state);
                        }
                        Err(_) => editor.document.thumbnails.stopped(ticket, key),
                    }
                }
                drop(pending);
                editor.document.thumbnails.update_memory();
                editor.advance_thumbnail_demand(&cx);
            });
        }) else {
            self.document.thumbnails.stopped(ticket, fallback_key);
            return;
        };
        self.document.thumbnail_work.tasks.push(Pending {
            ticket,
            generation,
            _task: task,
            _retire: retire,
        });
    }

    pub(in crate::editor) fn advance_thumbnail_demand(&mut self, cx: &UiContext<'_, Self>) {
        let (visible, hidden) = match &self.document.thumbnails.demand {
            Demand::None => return,
            Demand::Visible(visible) if self.fullscreen.is_none() => (visible.clone(), false),
            Demand::Visible(_) | Demand::Hidden => (Vec::new(), true),
        };

        let allowed = self.thumbnail_work_allowed();
        let eager =
            allowed && self.document.thumbnails.budgets.pressure == memory::Pressure::Normal;
        let idle = allowed
            && self.document.visit.canvas.grade.original.refining.is_none()
            && if self.document.visit.working_view.revision().is_some() {
                self.document.visit.working_view.idle()
            } else {
                self.document.visit.canvas.grade.original.idle()
            };
        let mut protected = Vec::new();
        for id in &visible {
            if let Ok(source) = self.document.state.project.source(*id)
                && let Ok(path) = self.document.source_path(*id)
                && let Some(key) = self.thumbnail_key(source, &path, true)
            {
                protected.push(key);
            }
        }
        if let Some((_, key)) = &self.document.thumbnails.pending {
            let visible = visible.contains(&key.source.id);
            let expected = self
                .document
                .project
                .source(key.source.id)
                .ok()
                .zip(self.document.source_path(key.source.id).ok())
                .and_then(|(source, path)| self.thumbnail_key(source, &path, visible));
            if expected.as_ref() != Some(key)
                || (!visible
                    && (!eager
                        || (self.document.thumbnails.pending_render && !idle)
                        || protected
                            .iter()
                            .any(|key| !self.document.thumbnails.cache.contains(key))))
            {
                self.document.thumbnails.cancel();
            }
        }
        self.document.thumbnails.visible_keys = protected
            .iter()
            .filter_map(|key| key.source.store_key())
            .collect();
        self.document.thumbnails.protect_cpu();
        self.document.thumbnails.protect_images(protected.clone());
        if self.document.thumbnails.services.is_none() {
            if self.document.thumbnails.gpu.is_some() {
                for key in protected {
                    if !self.document.thumbnails.cache.contains(&key) {
                        self.document
                            .thumbnails
                            .cache
                            .insert(key, Err("Thumbnail worker stopped".into()));
                    }
                }
            }
            return;
        }
        if !allowed || self.document.thumbnails.pending.is_some() {
            return;
        }
        let visible_indices: Vec<_> = self
            .document
            .project
            .order
            .iter()
            .enumerate()
            .filter_map(|(index, id)| visible.contains(id).then_some(index))
            .collect();
        let mut candidates: Vec<_> = (0..self.document.state.project.order.len())
            .filter(|index| visible_indices.contains(index) || eager)
            .collect();
        if !hidden {
            candidates.sort_by_key(|index| {
                (
                    !visible_indices.contains(index),
                    visible_indices
                        .iter()
                        .map(|visible| index.abs_diff(*visible))
                        .min()
                        .unwrap_or(0),
                    index.abs_diff(self.document.selected),
                )
            });
        }
        for index in candidates {
            let id = self.document.state.project.order[index];
            let source = self
                .document
                .project
                .source(id)
                .expect("thumbnail demand source");
            let path = self
                .document
                .source_path(id)
                .expect("thumbnail demand locator");
            let visible = visible.contains(&id);
            let Some(key) = self.thumbnail_key(source, &path, visible) else {
                continue;
            };
            if self.document.thumbnails.cache.contains(&key) {
                continue;
            }
            let render = visible || (!hidden && idle && self.document.thumbnails.can_retain(&key));
            if !render
                && self
                    .document
                    .thumbnails
                    .attempted_sources
                    .contains(&key.source)
            {
                continue;
            }
            let source = key.source.metadata_record(&path);
            self.request_thumbnail(
                key,
                path,
                source,
                render,
                if visible {
                    source_loading::Priority::Visible
                } else {
                    source_loading::Priority::Eager
                },
                cx,
            );
            break;
        }
    }
}
