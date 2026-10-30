use super::*;
use futures_channel::oneshot;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(in crate::editor) struct Operation {
    pub task: Option<async_executor::Task<()>>,
    pub apply: Option<oneshot::Sender<Draft>>,
}

impl Editor {
    pub(in crate::editor) fn propose_photo_crop(
        &mut self,
        photo: SourceId,
        cx: &UiContext<'_, Self>,
    ) {
        if self.busy
            || self.document.crop_edit().is_none()
            || self.document.visit.canvas.ready.is_none()
            || self.document.visit.current_source != Some(photo)
        {
            return;
        }
        let Some(regions) = self.prepare_regions(cx) else {
            return;
        };
        let task = cx.spawn(move |cx| async move {
            let prepared = regions.await;
            cx.update(|editor| {
                editor
                    .document
                    .crop_edit_mut()
                    .expect("live Crop action")
                    .auto_crop_task = None;
                if prepared.is_some() {
                    editor.auto_crop_photo(photo);
                }
            });
        });
        self.document.crop_edit_mut().unwrap().auto_crop_task = task;
    }

    pub(in crate::editor) fn review_crop_proposals(
        &mut self,
        photos: Vec<SourceId>,
        scope: &'static str,
        cx: &UiContext<'_, Self>,
    ) {
        if photos.is_empty() || !self.auto_crop_entry_available() {
            return;
        }
        if !self.prepare_tool() {
            return;
        }
        let Some(regions) = self.prepare_regions(cx) else {
            return;
        };
        self.document
            .set_auto_crop_operation(Some(Operation::default()));
        let task = cx.spawn(move |cx| async move {
            if regions.await.is_none() {
                cx.update(|editor| editor.document.set_auto_crop_operation(None));
                return;
            }
            cx.update(|editor| editor.begin_auto_crop_review(photos, scope));
            loop {
                let (apply, decision) = oneshot::channel();
                let reviewing = cx.update(|editor| {
                    if editor.document.auto_crop_edit().is_none() {
                        return false;
                    }
                    editor
                        .document
                        .auto_crop_operation_mut()
                        .expect("live Auto-crop action")
                        .apply = Some(apply);
                    true
                });
                if reviewing != Some(true) {
                    break;
                }
                let Ok(snapshot) = decision.await else {
                    break;
                };
                let accepted = cx.update(|editor| {
                    if editor.commit_auto_crop(&snapshot) {
                        editor.end_auto_crop_review(true);
                        true
                    } else {
                        false
                    }
                });
                if accepted == Some(true) {
                    break;
                }
            }
            cx.update(|editor| editor.document.set_auto_crop_operation(None));
        });
        if let Some(task) = task {
            self.document.auto_crop_operation_mut().unwrap().task = Some(task);
        } else {
            self.document.set_auto_crop_operation(None);
        }
    }
}
