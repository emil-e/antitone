use super::*;

#[cfg(test)]
#[path = "editor_thumbnail_workflow_tests.rs"]
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
    pub(in crate::editor) fn start_thumbnail_workflow(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
    ) {
        let Some(job) = self.thumbnails.request.take() else {
            return;
        };
        let ticket = job.ticket;
        let key = job.key.clone();
        let fallback_key = key.clone();
        let store = job.store.clone();
        let generation = self.thumbnails.generation.clone();
        let Some(services) = self.thumbnails.services.clone() else {
            self.thumbnails.stopped(ticket, key);
            return;
        };
        let identity = generation.clone();
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let result = services.process(job).await;
            target.update(|editor| {
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
                editor.advance_thumbnail_demand();
                editor.start_thumbnail_workflow(&target);
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

    pub(in crate::editor) fn advance_thumbnail_demand(&mut self) {
        let (visible, hidden) = match &self.thumbnails.demand {
            Demand::None => return,
            Demand::Visible(visible) if self.fullscreen.is_none() => (visible.clone(), false),
            Demand::Visible(_) | Demand::Hidden => (Vec::new(), true),
        };
        if self.document.cache().is_closed() {
            if self.thumbnails.pending.is_some() || self.thumbnails.request.is_some() {
                self.thumbnails.cancel();
            }
            return;
        }
        let allowed = self.thumbnail_work_allowed();
        let eager = allowed && self.thumbnails.budgets.pressure == memory::Pressure::Normal;
        let idle = allowed
            && self.workflow_jobs.idle()
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
            if self.thumbnails.cache.contains(&key)
                || (!visible && self.thumbnails.attempted_sources.contains(&key.source))
            {
                continue;
            }
            let render = visible
                || (!hidden
                    && idle
                    && key
                        .estimated_product_bytes()
                        .is_some_and(|bytes| self.thumbnails.cache.can_retain(bytes)));
            self.thumbnails.request_prioritized(
                key,
                path,
                source.clone(),
                self.correction.clone(),
                render,
                if visible {
                    development::Priority::Visible
                } else {
                    development::Priority::Eager
                },
            );
            break;
        }
    }
}
