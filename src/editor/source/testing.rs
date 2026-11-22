use super::*;
use futures_channel::oneshot;

pub(in crate::editor) fn complete(editor: &mut Editor, result: Result<Box<Ready>, String>) {
    editor.source_result(result);
}

pub(in crate::editor) fn pending(
    editor: &mut Editor,
    cx: &UiContext<'_, Editor>,
) -> oneshot::Sender<Result<Box<Ready>, String>> {
    let (reply, result) = oneshot::channel();
    editor.document.visit.source.task = cx.spawn(move |cx| async move {
        if let Ok(result) = result.await {
            cx.update(|editor| editor.source_result(result));
        }
    });
    reply
}
