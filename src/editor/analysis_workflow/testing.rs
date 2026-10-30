use super::*;
use std::pin::Pin;

pub(in crate::editor) type Work<T> = Box<
    dyn FnOnce(
        analysis::Request,
        Cancellation,
    ) -> Pin<Box<dyn Future<Output = Result<Option<T>, String>>>>,
>;

pub(in crate::editor) struct Reply<T> {
    pub result: oneshot::Sender<Result<Option<T>, String>>,
    pub request: oneshot::Receiver<analysis::Request>,
}

fn reply<T: 'static>() -> (Work<T>, Reply<T>) {
    let (result, receive) = oneshot::channel();
    let (capture, request) = oneshot::channel();
    let work: Work<T> = Box::new(move |request, _| {
        Box::pin(async move {
            let _ = capture.send(request);
            receive
                .await
                .unwrap_or_else(|_| Err("programmed analysis worker disconnected".into()))
        })
    });
    (work, Reply { result, request })
}

pub(in crate::editor) fn request<T>(
    app: &mut crate::application::test_support::TestApplication<'_, Editor>,
    reply: &mut Reply<T>,
) -> analysis::Request {
    let captured = std::cell::RefCell::new(None);
    let request = std::cell::RefCell::new(&mut reply.request);
    app.run_until(super::tests::LIMIT, |_| {
        *captured.borrow_mut() = request
            .borrow_mut()
            .try_recv()
            .expect("analysis worker input channel");
        captured.borrow().is_some()
    });
    captured.into_inner().unwrap()
}

pub(in crate::editor) fn preparation(editor: &mut Editor) -> Reply<analysis_review::Approved> {
    let (work, reply) = reply();
    editor.document.analysis_preparation_work = Some(work);
    reply
}

pub(in crate::editor) fn fitting(editor: &mut Editor) -> Reply<Box<analysis::Outcome>> {
    let (work, reply) = reply();
    editor.document.analysis_fit_work = Some(work);
    reply
}

impl Editor {
    pub(in crate::editor) fn prepare_analysis_review(
        &mut self,
        request: analysis::Request,
        cx: &UiContext<'_, Self>,
    ) {
        self.document
            .set_analysis_operation(Some(Operation::new(Phase::Review)));
        let (id, services) = self.begin_analysis_preparation();
        let cancelled = self
            .document
            .analysis_operation()
            .unwrap()
            .cancelled
            .clone();
        let retire = cancelled.cancel_on_drop();
        let task = cx.spawn(move |cx| async move {
            let _retire = retire;
            sampling_review(request, id, services, cancelled, cx).await;
        });
        self.document.analysis_operation_mut().unwrap().task = task;
    }

    pub(in crate::editor) fn prepared_analysis_review(
        &mut self,
        request: analysis::Request,
        approved: analysis_review::Approved,
        cx: &UiContext<'_, Self>,
    ) {
        let operation = Operation::new(Phase::Review);
        let cancelled = operation.cancelled.clone();
        let retire = cancelled.cancel_on_drop();
        self.document.set_analysis_operation(Some(operation));
        let decision = self.open_analysis_review(&request, Ok(Some(approved)), cx);
        let task = cx.spawn(move |cx| async move {
            let _retire = retire;
            await_review(request, decision, cancelled, &cx).await;
        });
        if let Some(operation) = self.document.analysis_operation_mut() {
            operation.task = task;
        }
    }

    pub(in crate::editor) fn start_analysis(
        &mut self,
        request: analysis::Request,
        cx: &UiContext<'_, Self>,
    ) {
        let operation = Operation::new(Phase::Review);
        let cancelled = operation.cancelled.clone();
        let retire = cancelled.cancel_on_drop();
        self.document.set_analysis_operation(Some(operation));
        let Some(fit) = self.begin_analysis_fit(request.document.project.order.len()) else {
            self.document.set_analysis_operation(None);
            return;
        };
        let task = cx.spawn(move |cx| async move {
            let _retire = retire;
            fit_and_accept(request, fit, cancelled, &cx).await;
        });
        self.document.analysis_operation_mut().unwrap().task = task;
    }
}
