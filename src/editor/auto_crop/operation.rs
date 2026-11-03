use super::*;
#[cfg(test)]
use futures_channel::oneshot;

#[cfg(test)]
mod tests;

pub(in crate::editor) fn propose_photo_crop(
    editor: &mut Editor,
    photo: SourceId,
    state: std::rc::Rc<std::cell::RefCell<CropEdit>>,
    cx: &UiContext<'_, Editor>,
) -> Option<async_executor::Task<()>> {
    if editor.busy
        || editor.document.visit.canvas.ready.is_none()
        || editor.document.visit.current_source != Some(photo)
    {
        return None;
    }
    editor.sync_document();
    let regions = editor.prepare_regions(cx)?;
    cx.spawn(move |cx| async move {
        if regions.await.is_none() {
            return;
        }
        let result = cx.read(|editor| editor.photo_crop_proposal(photo));
        let status = finish_photo_proposal(&mut state.borrow_mut(), result);
        cx.update(|editor| editor.status = status);
    })
}

impl Editor {
    pub(in crate::editor) fn review_crop_proposals(
        &mut self,
        photos: Vec<SourceId>,
        scope: &'static str,
        cx: &UiContext<'_, Self>,
    ) {
        if photos.is_empty() || !self.auto_crop_entry_available() || !self.prepare_tool() {
            return;
        }
        let Some(regions) = self.prepare_regions(cx) else {
            return;
        };
        let mut tool = AutoCropTool::new(None);
        let state = std::rc::Rc::downgrade(&tool.review);
        let Some(task) = cx.spawn(move |cx| async move {
            if regions.await.is_none() {
                cx.update(Editor::end_tool);
                return;
            }
            cx.update(|editor| {
                let Some(state) = state.upgrade() else { return };
                match editor.capture_crop_review(photos, scope) {
                    Some(review) => *state.borrow_mut() = Some(review),
                    None => editor.end_tool(),
                }
            });
        }) else {
            return;
        };
        tool.preparation = Some(task);
        tool.install(self);
    }
}
