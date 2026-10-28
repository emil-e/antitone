use super::*;
use crate::{
    cancellation::{CancelOnDrop, Cancellation},
    dust_repair::SearchOutcome,
};

#[derive(Clone)]
pub(crate) struct Request {
    pub id: u64,
    pub source: SourceId,
    pub previous: Option<Repair>,
    pub support: SupportMask,
    pub input: InputSettings,
    pub context: source::Context,
    pub development: crate::project::SourceDevelopment,
    pub cancelled: Cancellation,
}

struct Pending {
    request: Request,
    task: Option<async_executor::Task<()>>,
    _retire: CancelOnDrop,
}

#[derive(Default)]
pub(super) struct State {
    pending: Vec<Pending>,
    latest: HashMap<SourceId, u64>,
}

impl State {
    pub(super) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub(super) fn supports(&self, source: SourceId) -> impl Iterator<Item = &SupportMask> {
        self.pending
            .iter()
            .filter(move |p| p.request.source == source)
            .map(|p| &p.request.support)
    }

    pub(super) fn has_target(&self, source: SourceId, repair: RepairId) -> bool {
        self.pending.iter().any(|p| {
            p.request.source == source
                && p.request.previous.as_ref().is_some_and(|r| r.id == repair)
        })
    }

    pub(super) fn cancel_target(&mut self, source: SourceId, repair: RepairId) {
        let Some(index) = self.pending.iter().position(|p| {
            p.request.source == source
                && p.request.previous.as_ref().is_some_and(|r| r.id == repair)
        }) else {
            return;
        };
        let pending = self.pending.remove(index);
        if self.latest.get(&source) == Some(&pending.request.id) {
            self.latest.remove(&source);
        }
    }

    pub(super) fn cancel_source(&mut self, source: SourceId) {
        self.pending.retain(|p| p.request.source != source);
        self.latest.remove(&source);
    }

    pub(super) fn manual_selection(&mut self, source: SourceId) {
        self.latest.remove(&source);
    }
}

impl Editor {
    #[cfg(test)]
    pub(in crate::editor) fn dust_search_requests(&self) -> Vec<Request> {
        self.document
            .dust
            .searches
            .pending
            .iter()
            .map(|p| p.request.clone())
            .collect()
    }

    pub(in crate::editor) fn dust_search_pending(&self) -> bool {
        !self.document.dust.searches.is_empty()
    }

    pub(in crate::editor) fn queue_dust_search(
        &mut self,
        source: SourceId,
        previous: Option<Repair>,
        support: SupportMask,
    ) {
        let context = self
            .source_context()
            .expect("released stroke has a ready source");
        let request = Request {
            id: self.request_id(),
            source,
            previous,
            support,
            context,
            input: self.input(),
            development: self
                .document
                .project
                .source(source)
                .expect("stroke source exists")
                .development
                .clone(),
            cancelled: Cancellation::default(),
        };
        if request.previous.is_none() {
            self.document.dust.selected.remove(&source);
        }
        self.document
            .dust
            .searches
            .latest
            .insert(source, request.id);
        let retire = request.cancelled.cancel_on_drop();
        self.document.dust.searches.pending.push(Pending {
            request,
            task: None,
            _retire: retire,
        });
    }

    pub(in crate::editor) fn start_dust_searches(&mut self, cx: &UiContext<'_, Self>) {
        let queued = self
            .document
            .dust
            .searches
            .pending
            .iter()
            .filter(|p| p.task.is_none())
            .map(|p| p.request.clone())
            .collect::<Vec<_>>();
        for request in queued {
            let id = request.id;
            let services = self.dust_services.clone();
            let retire = request.cancelled.cancel_on_drop();
            let Some(task) = cx.spawn(move |cx| async move {
                let _retire = retire;
                let result = match services {
                    Some(services) => services.search(request).await,
                    None => Err("Dust services unavailable".into()),
                };
                cx.update(|e| {
                    e.receive_dust_search(id, result);
                    e.start_dust_searches(&cx);
                    if let Some(signature) = e.dust_signature() {
                        e.schedule_dust(signature, &cx);
                    }
                });
            }) else {
                self.receive_dust_search(id, Err("Application stopped".into()));
                continue;
            };
            if let Some(pending) = self
                .document
                .dust
                .searches
                .pending
                .iter_mut()
                .find(|p| p.request.id == id)
            {
                pending.task = Some(task);
            }
        }
    }

    pub(in crate::editor) fn receive_dust_search(
        &mut self,
        id: u64,
        result: Result<SearchOutcome, String>,
    ) {
        let Some(index) = self
            .document
            .dust
            .searches
            .pending
            .iter()
            .position(|p| p.request.id == id)
        else {
            return;
        };
        let request = self.document.dust.searches.pending[index].request.clone();
        let current = !request.cancelled.is_cancelled()
            && self.document_pending.is_none()
            && self
                .document
                .project
                .source(request.source)
                .is_ok_and(|source| {
                    source.development == request.development
                        && request
                            .previous
                            .as_ref()
                            .is_none_or(|previous| source.repairs.repairs().contains(previous))
                });
        let select = self.document.dust.searches.latest.get(&request.source) == Some(&id);
        self.document.dust.searches.pending.remove(index);
        if select {
            self.document.dust.searches.latest.remove(&request.source);
        }
        if !current {
            self.status = "Dust stroke cancelled: its source or repair changed".into();
            return;
        }
        let reference = self
            .document
            .project
            .source(request.source)
            .ok()
            .and_then(|source| self.document.exposure_value(source));
        let reference_changed =
            !self.input().is_negative() && request.context.reference != reference;
        let context_changed = request.input != self.input()
            || reference_changed
            || request.context.correction.as_ref().map(|c| &c.settings)
                != self.document.correction.as_ref().map(|c| &c.settings);
        if context_changed {
            // Keep the released support, but score it in the new interpretation.
            let mut retry = request;
            retry.id = self.request_id();
            retry.input = self.input();
            retry.context.correction = self.document.correction.clone();
            retry.context.reference = if retry.input.is_negative() {
                None
            } else {
                reference
            };
            retry.cancelled = Cancellation::default();
            if select {
                self.document
                    .dust
                    .searches
                    .latest
                    .insert(retry.source, retry.id);
            }
            let retire = retry.cancelled.cancel_on_drop();
            self.document.dust.searches.pending.push(Pending {
                request: retry,
                task: None,
                _retire: retire,
            });
            return;
        }
        let offset = match result {
            Ok(SearchOutcome::Candidate(offset)) => offset,
            Ok(SearchOutcome::Impossible) => {
                self.status =
                    "Dust stroke rejected: no clean donor placement; no changes applied".into();
                return;
            }
            Ok(SearchOutcome::Cancelled) => return,
            Err(error) => {
                self.status = format!("Dust stroke not accepted: {error}");
                return;
            }
        };
        let active = request.development.active_area.into();
        if let Err(error) = validate_dust_offset(offset, &request.support, active) {
            self.status = format!("Dust stroke not accepted: {error}");
            return;
        }
        let before = self.history_snapshot();
        let result = match request.previous {
            Some(previous) => self
                .dust_repairs_mut(request.source)
                .set_support(previous.id, request.support)
                .and_then(|()| {
                    self.dust_repairs_mut(request.source)
                        .set_offset(previous.id, offset)
                })
                .map(|()| previous.id),
            None => self.dust_repairs_mut(request.source).insert(
                request.support,
                RepairOrigin::Manual,
                offset,
            ),
        };
        match result {
            Ok(repair) => {
                if select && self.document.visit.dust.mask_draft.is_none() {
                    self.document.dust.selected.insert(request.source, repair);
                }
                self.dust_repair_changed(request.source, repair);
                self.record_edit(before, "Dust mask", Some(request.source));
                self.status = "Donor selected; evaluating repair…".into();
                self.invalidate_settled();
            }
            Err(error) => self.status = format!("Dust stroke not accepted: {error}"),
        }
    }
}
