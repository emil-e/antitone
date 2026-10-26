use super::*;

#[cfg(test)]
mod workflow_tests;

pub(super) struct Submission {
    pub job: Job,
    pub reply: futures_channel::oneshot::Sender<Outcome>,
}

#[derive(Clone)]
pub(super) struct Services {
    pub commands: mpsc::Sender<Submission>,
}

#[derive(Debug)]
pub(super) struct Stopped;

impl Services {
    pub async fn process(self, job: Job) -> Result<Outcome, Stopped> {
        let (reply, result) = futures_channel::oneshot::channel();
        self.commands
            .send(Submission { job, reply })
            .map_err(|_| Stopped)?;
        result.await.map_err(|_| Stopped)
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
        priority: development::Priority,
        cx: &UiContext<'_, Self>,
    ) {
        if self.thumbnails.pending.is_some() || self.thumbnails.cache.get(&key).is_some() {
            return;
        }
        let Some(services) = self.thumbnails.services.clone() else {
            if self.thumbnails.gpu.is_some() {
                self.thumbnails
                    .cache
                    .insert(key, Err("Thumbnail worker stopped".into()));
            }
            return;
        };
        let generation = self.thumbnails.generation.clone();
        let ticket = generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let previous = render
            .then(|| {
                self.thumbnails
                    .compatible_product(&key)
                    .map(|(_, product)| product.clone())
            })
            .flatten();
        let job = Job {
            ticket,
            key: key.clone(),
            path,
            source,
            correction: self.correction.clone(),
            store: self.thumbnails.sources.clone(),
            render,
            priority,
            previous,
        };
        self.thumbnails.pending_render = render;
        self.thumbnails.pending = Some((ticket, key.clone()));
        let fallback_key = key.clone();
        let store = job.store.clone();
        let identity = generation.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let result = services.process(job).await;
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
                if Arc::ptr_eq(&store, editor.document.cache())
                    && Arc::ptr_eq(&identity, &editor.thumbnails.generation)
                {
                    match result {
                        Ok(outcome) => editor.thumbnails.complete(outcome, &editor.document),
                        Err(_) => editor.thumbnails.stopped(ticket, key),
                    }
                }
                drop(pending);
                editor.thumbnails.update_memory();
                editor.advance_thumbnail_demand(&cx);
            });
        }) else {
            self.thumbnails.stopped(ticket, fallback_key);
            return;
        };
        self.document.thumbnail_work.tasks.push(Pending {
            ticket,
            generation,
            _task: task,
        });
    }

    pub(in crate::editor) fn advance_thumbnail_demand(&mut self, cx: &UiContext<'_, Self>) {
        let (visible, hidden) = match &self.thumbnails.demand {
            Demand::None => return,
            Demand::Visible(visible) if self.fullscreen.is_none() => (visible.clone(), false),
            Demand::Visible(_) | Demand::Hidden => (Vec::new(), true),
        };
        if self.document.cache().is_closed() {
            if self.thumbnails.pending.is_some() {
                self.thumbnails.cancel();
            }
            return;
        }
        let allowed = self.thumbnail_work_allowed();
        let eager = allowed && self.thumbnails.budgets.pressure == memory::Pressure::Normal;
        let idle = allowed
            && self.original.refining.is_none()
            && if self.working.revision().is_some() {
                self.working.idle()
            } else {
                self.original.idle()
            };
        let mut protected = Vec::new();
        for id in &visible {
            if let Ok(source) = self.document.project.source(*id)
                && let Ok(path) = self.document.source_path(*id)
                && let Some(key) = self.thumbnail_key(source, &path, true)
            {
                protected.push(key);
            }
        }
        if let Some((_, key)) = &self.thumbnails.pending {
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
                        || (self.thumbnails.pending_render && !idle)
                        || protected
                            .iter()
                            .any(|key| !self.thumbnails.cache.contains(key))))
            {
                self.thumbnails.cancel();
            }
        }
        self.thumbnails.visible_keys = protected
            .iter()
            .filter_map(|key| key.source.store_key())
            .collect();
        self.thumbnails.protect_cpu();
        self.thumbnails.protect_images(protected.clone());
        if self.thumbnails.services.is_none() {
            if self.thumbnails.gpu.is_some() {
                for key in protected {
                    if !self.thumbnails.cache.contains(&key) {
                        self.thumbnails
                            .cache
                            .insert(key, Err("Thumbnail worker stopped".into()));
                    }
                }
            }
            return;
        }
        if !allowed || self.thumbnails.pending.is_some() {
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
        let mut candidates: Vec<_> = (0..self.document.project.order.len())
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
                    index.abs_diff(self.selected),
                )
            });
        }
        for index in candidates {
            let id = self.document.project.order[index];
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
            if self.thumbnails.cache.contains(&key) {
                continue;
            }
            let render = visible || (!hidden && idle && self.thumbnails.can_retain(&key));
            if !render && self.thumbnails.attempted_sources.contains(&key.source) {
                continue;
            }
            let source = key.source.metadata_record(&path);
            self.request_thumbnail(
                key,
                path,
                source,
                render,
                if visible {
                    development::Priority::Visible
                } else {
                    development::Priority::Eager
                },
                cx,
            );
            break;
        }
    }
}
