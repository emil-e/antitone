//! Opt-in native idle-executor check; ordinary test invocations do not open a window.
use antitone::{
    application::UiContext,
    platform::{SubmissionGate, host},
};
use futures_channel::oneshot;
use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    task::Poll,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Report {
    paints: AtomicUsize,
    completed_paint: AtomicBool,
    modal_entered: AtomicBool,
    modal_finished: AtomicBool,
    cancelled: AtomicBool,
    exited: AtomicBool,
}

struct Paint {
    report: Arc<Report>,
    completed: bool,
    repaint: egui::Context,
}

impl egui_wgpu::CallbackTrait for Paint {
    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        _: &mut wgpu::RenderPass<'static>,
        _: &egui_wgpu::CallbackResources,
    ) {
        self.report.paints.fetch_add(1, Ordering::SeqCst);
        if self.completed {
            self.report.completed_paint.store(true, Ordering::SeqCst);
            self.repaint.request_repaint();
        }
    }
}

struct Cancelled {
    ui_thread: std::thread::ThreadId,
    report: Arc<Report>,
    _local: Rc<Cell<bool>>,
}

impl Drop for Cancelled {
    fn drop(&mut self) {
        assert_eq!(std::thread::current().id(), self.ui_thread);
        self.report.cancelled.store(true, Ordering::SeqCst);
    }
}

struct View {
    submissions: SubmissionGate,
    report: Arc<Report>,
    in_ui: Rc<Cell<bool>>,
    completed: bool,
    modal_requested: bool,
    task: Option<async_executor::Task<()>>,
    pending: Option<async_executor::Task<()>>,
}

impl host::App for View {
    fn submission_gate(&self) -> SubmissionGate {
        self.submissions.clone()
    }

    fn on_exit(&mut self) {
        assert!(
            self.report.cancelled.load(Ordering::SeqCst),
            "local tasks must retire before app cleanup"
        );
        self.report.exited.store(true, Ordering::SeqCst);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _access: &UiContext<'_, Self>) {
        self.in_ui.set(true);
        if std::mem::take(&mut self.modal_requested) {
            self.report.modal_entered.store(true, Ordering::SeqCst);
            // A native modal operation can pump AppKit while the outer UI/state
            // callback is still borrowed. Executor events must wait for its return.
            let until = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.2);
            objc2_foundation::NSRunLoop::currentRunLoop().runUntilDate(&until);
            self.report.modal_finished.store(true, Ordering::SeqCst);
        }
        ui.heading("Antitone native executor check");
        ui.label(if self.completed {
            "Completed on the UI thread"
        } else {
            "Waiting without periodic redraw"
        });
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            ui.ctx().content_rect(),
            Paint {
                report: self.report.clone(),
                completed: self.completed,
                repaint: ui.ctx().clone(),
            },
        ));
        if self.report.completed_paint.load(Ordering::SeqCst) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.in_ui.set(false);
    }
}

pub fn main() -> host::Result<()> {
    if !std::env::args().skip(1).any(|arg| arg == "--run") {
        println!(
            "Native executor not requested. Use cargo test --features ui --test native_executor -- --run"
        );
        return Ok(());
    }
    let (stop, stopped) = mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if stopped.recv_timeout(Duration::from_secs(15)).is_err() {
            eprintln!("native executor process deadline exceeded");
            std::process::exit(1);
        }
    });
    let report = Arc::new(Report::default());
    let worker_report = report.clone();
    let (first, first_result) = oneshot::channel();
    let (second, second_result) = oneshot::channel();
    let (modal, modal_result) = oneshot::channel();
    let (resumed, resumes) = mpsc::channel();
    let late_waker = Arc::new(Mutex::new(None::<std::task::Waker>));
    let late = late_waker.clone();
    let worker = std::thread::spawn(move || -> Result<(), &'static str> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if Instant::now() >= deadline {
                return Err("host never became idle after painting");
            }
            let paints = worker_report.paints.load(Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(150));
            if paints > 0 && worker_report.paints.load(Ordering::SeqCst) == paints {
                first.send(()).map_err(|_| "initial continuation lost")?;
                resumes
                    .recv_timeout(Duration::from_secs(3))
                    .map_err(|_| "idle continuation did not resume")?;
                // The first continuation does not change presentation. Merely resuming
                // logical work must not require or cause a UI frame.
                std::thread::sleep(Duration::from_millis(150));
                if worker_report.paints.load(Ordering::SeqCst) != paints {
                    return Err("executor progress depended on or scheduled a paint");
                }
                second
                    .send(())
                    .map_err(|_| "state-update continuation lost")?;
                while !worker_report.modal_entered.load(Ordering::SeqCst) {
                    if Instant::now() >= deadline {
                        return Err("native nested loop never entered");
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                modal
                    .send(())
                    .map_err(|_| "nested-loop continuation lost")?;
                return Ok(());
            }
        }
    });
    let view_report = report.clone();
    let result = host::run(
        "Antitone executor test",
        move |_, _, access: UiContext<'_, View>| {
            let in_ui = Rc::new(Cell::new(false));
            let mut view = View {
                submissions: SubmissionGate::default(),
                report: view_report.clone(),
                in_ui: in_ui.clone(),
                completed: false,
                modal_requested: false,
                task: None,
                pending: None,
            };
            let target = access.clone();
            let ui_thread = std::thread::current().id();
            view.task = access.spawn(move |_| async move {
                first_result.await.unwrap();
                assert_eq!(std::thread::current().id(), ui_thread);
                assert!(!in_ui.get(), "executor re-entered UI presentation");
                resumed.send(()).unwrap();
                second_result.await.unwrap();
                assert_eq!(std::thread::current().id(), ui_thread);
                assert!(!in_ui.get());
                target.update(|view| view.modal_requested = true).unwrap();
                modal_result.await.unwrap();
                assert_eq!(std::thread::current().id(), ui_thread);
                assert!(!in_ui.get(), "executor re-entered a native nested UI loop");
                target
                    .update(|view| {
                        assert!(view.report.modal_finished.load(Ordering::SeqCst));
                        view.completed = true;
                    })
                    .unwrap();
            });
            let guard = Cancelled {
                ui_thread,
                report: view_report,
                _local: Rc::new(Cell::new(false)),
            };
            view.pending = access.spawn(move |_| async move {
                let _guard = guard;
                std::future::poll_fn(move |cx| {
                    *late.lock().unwrap() = Some(cx.waker().clone());
                    Poll::<()>::Pending
                })
                .await;
            });
            Ok(view)
        },
    );
    let worker_result = worker
        .join()
        .map_err(|_| "native executor worker panicked")?;
    // Exercise a foreign wake after executor, application, window and event-loop teardown.
    let waker = late_waker
        .lock()
        .unwrap()
        .take()
        .ok_or("pending task was never polled")?;
    std::thread::spawn(move || waker.wake())
        .join()
        .map_err(|_| "late wake panicked")?;
    stop.send(()).unwrap();
    watchdog.join().unwrap();
    let presented = result?;
    worker_result?;
    if presented == 0
        || !report.completed_paint.load(Ordering::SeqCst)
        || !report.exited.load(Ordering::SeqCst)
    {
        return Err("executor/application/presentation lifecycle incomplete".into());
    }
    println!(
        "native executor completed: idle main-thread continuation without paint, state update/repaint, nested-loop deferral, cancellation before cleanup, late foreign wake"
    );
    Ok(())
}
