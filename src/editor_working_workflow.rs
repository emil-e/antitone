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
        let jobs = services.jobs.clone();
        jobs.run(move || {
            if retired.load(Ordering::Relaxed) {
                return None;
            }
            working_view::acquire(
                &self.mailbox,
                &services.renderers,
                &mut self.kernels.lock().unwrap(),
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
    pub(super) fn start_working_workflow(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
    ) {
        if self.document.visit.working.task.is_some()
            || self.source_epoch.load(Ordering::Relaxed) != self.id
        {
            return;
        }
        let Some(request) = self.working.acquisition(self.id) else {
            return;
        };
        let Some(services) = self.source_services.clone() else {
            self.working
                .acquisition_failed("Working-source services unavailable".into());
            return;
        };
        let visit = self.document.visit.view;
        let epoch = self.source_epoch.clone();
        let submissions = self.submissions.clone();
        let mailbox = request.mailbox.clone();
        let retired = Arc::new(AtomicBool::new(false));
        let retire = Retire(retired.clone());
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let _retire = retire;
            let acquisition = request.prepare(services, submissions, epoch, retired);
            let support = async {
                loop {
                    let changed = mailbox.1.listen();
                    target.update(|editor| {
                        if editor.document.visit.view == visit {
                            editor.working.refresh_support();
                        }
                    });
                    changed.await;
                }
            };
            let result = futures_lite::future::or(acquisition, support).await;
            target.update(|editor| {
                if editor.document.visit.view != visit {
                    return;
                }
                editor.document.visit.working.task = None;
                match result {
                    Ok(Some(completion)) => editor.working.acquired(completion),
                    Ok(None) => {}
                    Err(error) => editor.working.acquisition_failed(error),
                }
                editor.start_working_workflow(&target);
            });
        }) else {
            self.working
                .acquisition_failed("Application stopped".into());
            return;
        };
        self.document.visit.working.task = Some(task);
    }
}
