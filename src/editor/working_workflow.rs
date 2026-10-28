use super::{
    Editor, source,
    working_view::{self, Acquisition, Completion},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(super) struct State {
    pub task: Option<async_executor::Task<()>>,
}

struct Retire(Arc<AtomicBool>);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Acquisition {
    pub async fn prepare(
        self,
        services: source::Services,
        submissions: crate::platform::SubmissionGate,
        epoch: Arc<std::sync::atomic::AtomicU64>,
        retired: Arc<AtomicBool>,
    ) -> Result<Option<Completion>, String> {
        crate::background::run(move || {
            if retired.load(Ordering::Relaxed) {
                return None;
            }
            working_view::acquire(
                &self.mailbox,
                &services.renderers,
                &self.kernels,
                &services.device,
                &services.queue,
                &submissions,
                &epoch,
                &retired,
            )
        })
        .await
        .map_err(|_| "Working-source acquisition worker panicked".to_owned())
    }
}

impl Editor {
    pub(super) fn request_working(
        &mut self,
        spec: crate::gpu::view::ViewSourceSpec,
        input: source::Context,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        self.document.visit.working_view.request(spec, input);
        self.acquire_working(cx);
    }

    pub(super) fn acquire_working(&mut self, cx: &crate::application::UiContext<'_, Self>) {
        if self.allow_close
            || self.document.visit.working.task.is_some()
            || self.source_epoch.load(Ordering::Relaxed) != self.id
        {
            return;
        }
        let Some(request) = self.document.visit.working_view.acquisition(self.id) else {
            return;
        };
        let Some(services) = self.source_services.clone() else {
            self.document
                .visit
                .working_view
                .acquisition_failed("Working-source services unavailable".into());
            return;
        };
        let visit = self.document.visit.view;
        let epoch = self.source_epoch.clone();
        let submissions = self.submissions.clone();
        let mailbox = request.mailbox.clone();
        let retired = Arc::new(AtomicBool::new(false));
        let retire = Retire(retired.clone());
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let acquisition = request.prepare(services, submissions, epoch, retired);
            let support = async {
                loop {
                    let changed = mailbox.1.listen();
                    cx.update(|editor| {
                        if editor.document.visit.view == visit {
                            editor.document.visit.working_view.refresh_support();
                        }
                    });
                    changed.await;
                }
            };
            let result = futures_lite::future::or(acquisition, support).await;
            cx.update(|editor| {
                if editor.document.visit.view != visit {
                    return;
                }
                editor.document.visit.working.task = None;
                match result {
                    Ok(Some(completion)) => editor.document.visit.working_view.acquired(completion),
                    Ok(None) => {}
                    Err(error) => editor.document.visit.working_view.acquisition_failed(error),
                }
                editor.acquire_working(&cx);
            });
        }) else {
            self.document
                .visit
                .working_view
                .acquisition_failed("Application stopped".into());
            return;
        };
        self.document.visit.working.task = Some(task);
    }
}
