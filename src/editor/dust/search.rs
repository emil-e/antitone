use super::*;
use crate::dust_repair::SearchOutcome;
use std::{cell::Cell, rc::Rc};

#[cfg(test)]
mod tests;

/// Released support remains visible and blocks overlapping edits while its task runs.
#[derive(Clone)]
pub(in crate::editor) struct PendingStroke {
    pub source: SourceId,
    pub support: SupportMask,
    target: Option<RepairId>,
    select: Cell<bool>,
}

pub(super) struct Input {
    pub support: SupportMask,
    pub previous_offset: Option<[i32; 2]>,
    pub input: InputSettings,
    pub context: source::Context,
    #[cfg(test)]
    pub worker: Option<worker::testing::Search>,
}

type Applied = Result<Option<RepairId>, String>;

struct Pending {
    stroke: Rc<PendingStroke>,
    task: async_executor::Task<Applied>,
}

#[derive(Default)]
pub(super) struct State {
    tasks: Vec<Pending>,
}

impl State {
    fn pending(&self) -> impl Iterator<Item = &Pending> {
        self.tasks
            .iter()
            .filter(|pending| !pending.task.is_finished())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pending().next().is_none()
    }

    // Outcomes are applied before task completion; pruning only releases finished handles.
    pub(super) fn prune(&mut self) {
        self.tasks.retain(|pending| !pending.task.is_finished());
    }

    pub(super) fn supports(&self, source: SourceId) -> impl Iterator<Item = &SupportMask> {
        self.pending()
            .filter(move |pending| pending.stroke.source == source)
            .map(|pending| &pending.stroke.support)
    }

    pub(super) fn has_target(&self, source: SourceId, repair: RepairId) -> bool {
        self.pending()
            .any(|pending| pending.stroke.source == source && pending.stroke.target == Some(repair))
    }

    pub(super) fn cancel_target(&mut self, source: SourceId, repair: RepairId) {
        self.tasks.retain(|pending| {
            pending.stroke.source != source || pending.stroke.target != Some(repair)
        });
    }

    pub(super) fn manual_selection(&self, source: SourceId) {
        for pending in self
            .pending()
            .filter(|pending| pending.stroke.source == source)
        {
            pending.stroke.select.set(false);
        }
    }
}

async fn apply_stroke(
    input: Input,
    stroke: Rc<PendingStroke>,
    active: SourceRegion,
    cx: UiContext<'_, Editor>,
) -> Applied {
    let choice = worker::search(input).await;
    cx.update(|editor| {
        let result = match choice {
            Ok(SearchOutcome::Candidate(offset)) => {
                validate_dust_offset(offset, &stroke.support, active)
                    .map_err(|error| error.to_string())
                    .and_then(|()| editor.accept_donor(&stroke, offset))
                    .map(Some)
            }
            Ok(SearchOutcome::Impossible) => {
                editor.status =
                    "Dust stroke rejected: no clean donor placement; no changes applied".into();
                Ok(None)
            }
            Ok(SearchOutcome::Cancelled) => Ok(None),
            Err(error) => Err(error),
        };
        if let Err(error) = &result {
            editor.status = format!("Dust stroke not accepted: {error}");
        }
        if let Some(signature) = editor.dust_signature() {
            editor.schedule_dust(signature, &cx);
        }
        result
    })
    .unwrap_or(Ok(None))
}

impl Editor {
    #[cfg(test)]
    pub(in crate::editor) fn pending_dust_strokes(&self) -> Vec<PendingStroke> {
        self.document
            .visit
            .dust
            .searches
            .pending()
            .map(|pending| (*pending.stroke).clone())
            .collect()
    }

    pub(in crate::editor) fn dust_search_pending(&self) -> bool {
        !self.document.visit.dust.searches.is_empty()
    }

    pub(super) fn find_dust_donor(
        &mut self,
        source: SourceId,
        previous: Option<Repair>,
        support: SupportMask,
        cx: &UiContext<'_, Self>,
    ) {
        let stroke = Rc::new(PendingStroke {
            source,
            target: previous.as_ref().map(|repair| repair.id),
            support: support.clone(),
            select: Cell::new(true),
        });
        let input = Input {
            support,
            previous_offset: previous.as_ref().map(|repair| repair.offset),
            input: self.input(),
            context: self
                .source_context()
                .expect("released stroke has a ready source"),
            #[cfg(test)]
            worker: self.document.dust.search_worker.clone(),
        };
        let active = self.document.visit.canvas.ready.as_ref().unwrap().area;
        if previous.is_none() {
            self.document.dust.selected.remove(&source);
        }
        self.document.visit.dust.searches.prune();
        self.document.visit.dust.searches.manual_selection(source);
        let model = stroke.clone();
        match cx.spawn(move |cx| apply_stroke(input, stroke, active, cx)) {
            Some(task) => self.document.visit.dust.searches.tasks.push(Pending {
                stroke: model,
                task,
            }),
            None => self.status = "Dust stroke not accepted: Application stopped".into(),
        }
    }

    fn accept_donor(
        &mut self,
        stroke: &PendingStroke,
        offset: [i32; 2],
    ) -> Result<RepairId, String> {
        let source = stroke.source;
        let before = self.history_snapshot();
        let repair = match stroke.target {
            Some(target) => self
                .dust_repairs_mut(source)
                .set_support(target, stroke.support.clone())
                .and_then(|()| self.dust_repairs_mut(source).set_offset(target, offset))
                .map(|()| target),
            None => self.dust_repairs_mut(source).insert(
                stroke.support.clone(),
                RepairOrigin::Manual,
                offset,
            ),
        }
        .map_err(|error| error.to_string())?;
        if stroke.select.get() && self.document.visit.dust.mask_draft.is_none() {
            self.document.dust.selected.insert(source, repair);
        }
        self.dust_repair_changed(source, repair);
        self.record_edit(before, "Dust mask", Some(source));
        self.status = "Donor selected; evaluating repair…".into();
        self.invalidate_settled();
        Ok(repair)
    }
}
