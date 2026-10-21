//! Explicit native termination negotiation. --run uses only isolated child apps;
//! it never requests logout/shutdown or changes personal projects/preferences.
use antitone::platform::{
    SubmissionGate,
    host::{self, QuitState},
};
use objc2::{
    ClassType, MainThreadMarker,
    runtime::{AnyObject, ClassBuilder, NSObject, Sel},
    sel,
};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSArray, NSObjectNSDelayedPerforming, NSRunLoopCommonModes};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Approve,
    CancelRetry,
    BlockedRetry,
    Interrupted,
    FullscreenApprove,
    NativeModal,
    PanicPending,
    CleanupPanic,
    ExistingHandler,
}

const MODES: [Mode; 9] = [
    Mode::Approve,
    Mode::CancelRetry,
    Mode::BlockedRetry,
    Mode::Interrupted,
    Mode::FullscreenApprove,
    Mode::NativeModal,
    Mode::PanicPending,
    Mode::CleanupPanic,
    Mode::ExistingHandler,
];

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::CancelRetry => "cancel-retry",
            Self::BlockedRetry => "blocked-retry",
            Self::Interrupted => "interrupted",
            Self::FullscreenApprove => "fullscreen-approve",
            Self::NativeModal => "native-modal",
            Self::PanicPending => "panic-pending",
            Self::CleanupPanic => "cleanup-panic",
            Self::ExistingHandler => "existing-handler",
        }
    }
    fn parse(value: &str) -> Self {
        MODES
            .into_iter()
            .find(|mode| mode.name() == value)
            .expect("known native termination case")
    }
}

fn record(path: &Path, message: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{message}").unwrap();
}

fn application() -> objc2::rc::Retained<NSApplication> {
    NSApplication::sharedApplication(MainThreadMarker::new().unwrap())
}

fn delegate_identity() -> (usize, usize) {
    let delegate = application().delegate().unwrap();
    let object: &AnyObject = (*delegate).as_ref();
    let will_terminate = object
        .class()
        .instance_method(sel!(applicationWillTerminate:))
        .unwrap();
    (
        object as *const AnyObject as usize,
        will_terminate.implementation() as *const () as usize,
    )
}

fn termination_entry() -> usize {
    let app = application();
    let object: &AnyObject = app.as_ref();
    object
        .class()
        .instance_method(sel!(terminate:))
        .unwrap()
        .implementation() as *const () as usize
}

extern "C-unwind" fn incompatible_handler(
    _: *mut AnyObject,
    _: Sel,
    _: *mut NSApplication,
) -> objc2_app_kit::NSApplicationTerminateReply {
    objc2_app_kit::NSApplicationTerminateReply::TerminateCancel
}

fn install_existing_handler() {
    let delegate = application().delegate().unwrap();
    let object: &AnyObject = (*delegate).as_ref();
    let selector = sel!(applicationShouldTerminate:);
    let mut builder =
        ClassBuilder::new(c"AntitoneTestTerminationHandler", NSObject::class()).unwrap();
    // Model a future toolkit implementation with the real optional-method ABI.
    unsafe {
        builder.add_method(
            selector,
            incompatible_handler
                as extern "C-unwind" fn(
                    *mut AnyObject,
                    Sel,
                    *mut NSApplication,
                )
                    -> objc2_app_kit::NSApplicationTerminateReply,
        );
        let source = builder.register().instance_method(selector).unwrap();
        assert!(
            objc2::ffi::class_addMethod(
                (object.class() as *const objc2::runtime::AnyClass).cast_mut(),
                selector,
                source.implementation(),
                objc2::ffi::method_getTypeEncoding(source)
            )
            .as_bool()
        );
    }
}

fn terminate_after(delay: f64) {
    let app = application();
    // Schedule outside winit's Rust event-handler borrow. Use common modes so
    // retries also run while AppKit is awaiting a deferred termination reply.
    unsafe {
        let modes = NSArray::from_slice(&[NSRunLoopCommonModes]);
        app.performSelector_withObject_afterDelay_inModes(sel!(terminate:), None, delay, &modes);
    }
}

struct PaintAck(Arc<AtomicUsize>);
impl egui_wgpu::CallbackTrait for PaintAck {
    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        _: &mut wgpu::RenderPass<'static>,
        _: &egui_wgpu::CallbackResources,
    ) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct NativeApp {
    submissions: SubmissionGate,
    mode: Mode,
    marker: PathBuf,
    identity: (usize, usize),
    state: QuitState,
    requests: usize,
    started: bool,
    retry: bool,
    requested_at: Instant,
    paints: Arc<AtomicUsize>,
    before_request: usize,
    fullscreen_requested: bool,
    fullscreen_painted_after: Option<usize>,
    presentation: objc2_app_kit::NSApplicationPresentationOptions,
}

impl host::App for NativeApp {
    fn submission_gate(&self) -> SubmissionGate {
        self.submissions.clone()
    }
    fn quit_state(&self) -> Option<QuitState> {
        Some(self.state)
    }
    fn request_quit(&mut self) {
        self.requests += 1;
        record(&self.marker, &format!("request:{}", self.requests));
        assert!(
            self.requests
                <= if matches!(self.mode, Mode::CancelRetry | Mode::BlockedRetry) {
                    2
                } else {
                    1
                }
        );
        self.requested_at = Instant::now();
        self.before_request = self.paints.load(Ordering::SeqCst);
        self.state = if self.mode == Mode::BlockedRetry && self.requests == 1 {
            record(&self.marker, "blocked");
            QuitState::Running
        } else {
            QuitState::Pending
        };
    }
    fn photo_fullscreen(&self) -> bool {
        self.fullscreen_requested
    }
    fn on_exit(&mut self) {
        if self.mode == Mode::FullscreenApprove {
            assert!(
                self.fullscreen_painted_after.is_some(),
                "fullscreen entry was never observed"
            );
            assert!(
                self.started,
                "fullscreen must paint before termination starts"
            );
        }
        assert_eq!(
            application().presentationOptions(),
            self.presentation,
            "system presentation was not restored before cleanup"
        );
        record(&self.marker, "cleanup");
        if self.mode == Mode::CleanupPanic {
            record(&self.marker, "panic");
            panic!("fixture panic before native termination approval");
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _access: &antitone::application::UiContext<'_, Self>) {
        assert_eq!(
            delegate_identity(),
            self.identity,
            "winit's delegate or terminal callback was replaced"
        );
        ui.heading("Native termination negotiation check");
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            ui.ctx().content_rect(),
            PaintAck(self.paints.clone()),
        ));
        if !self.started {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
            if self.paints.load(Ordering::SeqCst) == 0 || application().keyWindow().is_none() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                ui.ctx().request_repaint_after(Duration::from_millis(16));
                return;
            }
            if self.mode == Mode::FullscreenApprove {
                if !self.fullscreen_requested {
                    self.fullscreen_requested = true;
                    return;
                }
                let app = application();
                let covers_display = app.keyWindow().is_some_and(|window| {
                    window.screen().is_some_and(|screen| {
                        let frame = window.frame();
                        let target = screen.frame();
                        (frame.origin.x - target.origin.x).abs() <= 1.
                            && (frame.origin.y - target.origin.y).abs() <= 1.
                            && (frame.size.width - target.size.width).abs() <= 1.
                            && (frame.size.height - target.size.height).abs() <= 1.
                            && window.isOnActiveSpace()
                            && !window
                                .styleMask()
                                .contains(objc2_app_kit::NSWindowStyleMask::FullScreen)
                    })
                });
                let presentation = objc2_app_kit::NSApplicationPresentationOptions::AutoHideDock
                    | objc2_app_kit::NSApplicationPresentationOptions::AutoHideMenuBar;
                if !covers_display || app.presentationOptions() != presentation {
                    return;
                }
                let paints = self.paints.load(Ordering::SeqCst);
                let Some(before) = self.fullscreen_painted_after else {
                    self.fullscreen_painted_after = Some(paints);
                    return;
                };
                if paints <= before {
                    return;
                }
            }
            self.started = true;
            record(&self.marker, "ready");
            if self.mode == Mode::FullscreenApprove {
                record(&self.marker, "fullscreen");
            }
            if self.mode == Mode::NativeModal {
                antitone::platform::menu::with_native_dialog(|| application().terminate(None));
                assert_eq!(
                    self.requests, 0,
                    "native modal owns input and cannot accept an editor quit request"
                );
                record(&self.marker, "modal-veto");
            }
            terminate_after(0.);
            if self.mode == Mode::CancelRetry {
                terminate_after(0.005);
                terminate_after(0.010);
            }
        }
        if self.state == QuitState::Pending
            && self.paints.load(Ordering::SeqCst) >= self.before_request + 2
            && self.requested_at.elapsed() >= Duration::from_millis(50)
        {
            if self.mode == Mode::CancelRetry && self.requests == 1 {
                record(&self.marker, "cancel");
                self.state = QuitState::Running;
            } else if self.mode == Mode::PanicPending {
                record(&self.marker, "panic");
                panic!("fixture panic while native termination is deferred");
            } else if self.mode == Mode::Interrupted {
                record(&self.marker, "interrupted");
                // Exiting the host is not itself permission to approve a native request.
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                record(&self.marker, "approve");
                self.state = QuitState::Approved;
            }
        }
        if self.requests == 1 && self.state == QuitState::Running && !self.retry {
            self.retry = true;
            terminate_after(0.1);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

impl Drop for NativeApp {
    fn drop(&mut self) {
        record(&self.marker, "drop");
    }
}

fn child(mode: Mode, marker: PathBuf) -> host::Result<()> {
    let result = marker.clone();
    let entry = Arc::new(AtomicUsize::new(0));
    let original = entry.clone();
    let outcome = host::run("Antitone termination check", move |_, _, _application| {
        original.store(termination_entry(), Ordering::SeqCst);
        if mode == Mode::ExistingHandler {
            install_existing_handler();
        }
        Ok(NativeApp {
            submissions: SubmissionGate::default(),
            mode,
            marker,
            identity: delegate_identity(),
            state: QuitState::Running,
            requests: 0,
            started: false,
            retry: false,
            requested_at: Instant::now(),
            paints: Arc::new(AtomicUsize::new(0)),
            before_request: 0,
            fullscreen_requested: false,
            fullscreen_painted_after: None,
            presentation: application().presentationOptions(),
        })
    });
    if mode == Mode::ExistingHandler {
        let error = outcome.expect_err("must not overwrite an existing termination decision");
        assert!(
            error
                .to_string()
                .contains("already implements applicationShouldTerminate")
        );
        assert_eq!(termination_entry(), entry.load(Ordering::SeqCst));
        record(&result, "rejected");
        return Ok(());
    }
    outcome?;
    assert_eq!(
        termination_entry(),
        entry.load(Ordering::SeqCst),
        "native termination entry was not restored on host return"
    );
    record(&result, "returned");
    if mode != Mode::Interrupted {
        return Err("AppKit affirmative termination unexpectedly returned to the caller".into());
    }
    Ok(())
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run() -> host::Result<()> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".cache")
        .join(format!(
            "native-termination-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        ));
    std::fs::create_dir_all(&directory)?;
    let scratch = Scratch(directory);
    for mode in MODES {
        let marker = scratch.0.join(mode.name());
        let stdout = std::fs::File::create(scratch.0.join(format!("{}.stdout", mode.name())))?;
        let stderr = std::fs::File::create(scratch.0.join(format!("{}.stderr", mode.name())))?;
        let mut process = Command::new(std::env::current_exe()?)
            .arg("--child")
            .arg(mode.name())
            .arg(&marker)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()?;
        let start = Instant::now();
        let status = loop {
            if let Some(status) = process.try_wait()? {
                break status;
            }
            if start.elapsed() > Duration::from_secs(20) {
                process.kill()?;
                process.wait()?;
                let output =
                    std::fs::read_to_string(scratch.0.join(format!("{}.stdout", mode.name())))
                        .unwrap_or_default();
                let errors =
                    std::fs::read_to_string(scratch.0.join(format!("{}.stderr", mode.name())))
                        .unwrap_or_default();
                let events = std::fs::read_to_string(&marker).unwrap_or_default();
                return Err(format!(
                    "native termination {} timed out\n{output}\n{errors}\nevents:\n{events}",
                    mode.name()
                )
                .into());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let expected_code = if matches!(mode, Mode::PanicPending | Mode::CleanupPanic) {
            101
        } else {
            0
        };
        if status.code() != Some(expected_code) {
            let diagnostics =
                std::fs::read_to_string(scratch.0.join(format!("{}.stderr", mode.name())))?;
            return Err(format!(
                "native termination {} failed: {status}\n{diagnostics}",
                mode.name()
            )
            .into());
        }
        let events = std::fs::read_to_string(marker)?;
        let expected = match mode {
            Mode::Approve => "ready\nrequest:1\napprove\ncleanup\ndrop\n",
            Mode::FullscreenApprove => "ready\nfullscreen\nrequest:1\napprove\ncleanup\ndrop\n",
            Mode::CancelRetry => "ready\nrequest:1\ncancel\nrequest:2\napprove\ncleanup\ndrop\n",
            Mode::BlockedRetry => "ready\nrequest:1\nblocked\nrequest:2\napprove\ncleanup\ndrop\n",
            Mode::Interrupted => "ready\nrequest:1\ninterrupted\ncleanup\ndrop\nreturned\n",
            Mode::NativeModal => "ready\nmodal-veto\nrequest:1\napprove\ncleanup\ndrop\n",
            Mode::PanicPending => "ready\nrequest:1\npanic\ndrop\n",
            Mode::CleanupPanic => "ready\nrequest:1\napprove\ncleanup\npanic\ndrop\n",
            Mode::ExistingHandler => "drop\nrejected\n",
        };
        assert_eq!(
            events,
            expected,
            "termination decision/cleanup order for {}",
            mode.name()
        );
        println!("native termination {}: passed", mode.name());
    }
    Ok(())
}

fn main() -> host::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--child") {
        child(Mode::parse(&args[2]), PathBuf::from(&args[3]))
    } else if args.iter().any(|arg| arg == "--run") {
        run()
    } else {
        println!("Native termination requires explicit --run and a macOS desktop.");
        Ok(())
    }
}
