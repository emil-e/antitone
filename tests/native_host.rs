//! Explicit native host lifecycle check; ordinary Cargo test invocations never open its window.
//! Run with --run [lifecycle|fullscreen|editor|early-close|timeout|process-timeout]. Requires macOS/Metal and a desktop.
//! The editor case additionally needs the editor feature/native setup and uses an authored temporary project.
//! Checks focus/size forwarding, native resize, submitted frames, clean exit and explicit completion.
//! Early close, timeout, absent completion or zero frames fail. A process watchdog bounds hangs.
//! Does not establish physical gestures, native dialogs, monitor colour or broader-device acceptance.
use antitone::platform::{SubmissionGate, host};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

#[cfg(feature = "editor")]
#[path = "native_host/editor.rs"]
mod editor;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Lifecycle,
    Fullscreen,
    Editor,
    EarlyClose,
    Timeout,
    ProcessTimeout,
}

#[derive(Default)]
struct Report {
    completed: bool,
    timed_out: bool,
    exited: bool,
    focused: bool,
    sizes: usize,
    painted: [bool; SIZES.len()],
    fullscreen_entered: bool,
    fullscreen_returned: bool,
    presentation_restored: bool,
    desktop_rect: Option<egui::Rect>,
    presentation_before: Option<objc2_app_kit::NSApplicationPresentationOptions>,
}

impl Report {
    fn validate(&self, presented: u64) -> Result<(), &'static str> {
        if self.timed_out {
            return Err("native host timed out");
        }
        if !self.completed {
            return Err("native host interrupted before completion");
        }
        if !self.exited {
            return Err("native host did not cleanly exit");
        }
        if !self.focused || self.sizes != SIZES.len() {
            return Err("native focus/resize forwarding incomplete");
        }
        if presented == 0 {
            return Err("native host presented zero frames");
        }
        if !self.painted.into_iter().all(|painted| painted) || presented < SIZES.len() as u64 {
            return Err("native host did not paint and present every requested size");
        }
        Ok(())
    }

    fn validate_fullscreen(&self, presented: u64) -> Result<(), &'static str> {
        self.validate(presented)?;
        if !(self.fullscreen_entered && self.fullscreen_returned && self.presentation_restored) {
            return Err("photo-only fullscreen round trip incomplete");
        }
        Ok(())
    }
}

const SIZES: [[f32; 2]; 4] = [[720., 320.], [800., 600.], [1100., 800.], [720., 320.]];

struct PaintAck {
    report: Arc<Mutex<Report>>,
    index: usize,
}

impl egui_wgpu::CallbackTrait for PaintAck {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        _pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        self.report.lock().unwrap().painted[self.index] = true;
    }
}

struct NativeApp {
    submissions: SubmissionGate,
    report: Arc<Mutex<Report>>,
    mode: Mode,
    start: Instant,
    requested: bool,
    index: usize,
}

impl host::App for NativeApp {
    fn submission_gate(&self) -> SubmissionGate {
        self.submissions.clone()
    }

    fn photo_fullscreen(&self) -> bool {
        self.mode == Mode::Fullscreen && self.index == 1
    }

    fn on_exit(&mut self) {
        self.report.lock().unwrap().exited = true;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _access: &antitone::application::UiAccess<'_, Self>) {
        ui.heading("Antitone native host test");
        ui.label("Resize/focus forwarding and explicit close completion; no personal state.");
        if ui.input(|input| input.viewport().close_requested()) {
            return;
        }
        if !self.requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
            self.requested = true;
        }
        let deadline = if self.mode == Mode::Timeout {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(10)
        };
        if self.start.elapsed() >= deadline {
            eprintln!(
                "native deadline: stage={} viewport={:?} painted={:?}",
                self.index,
                ui.input(|input| input.viewport().clone()),
                self.report.lock().unwrap().painted
            );
            self.report.lock().unwrap().timed_out = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.mode == Mode::EarlyClose {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let (size, focused) = ui.input(|input| {
            (
                input.viewport().inner_rect.map(|r| r.size()),
                input.viewport().focused == Some(true),
            )
        });
        let mut report = self.report.lock().unwrap();
        report.focused |= focused;
        if self.mode == Mode::Fullscreen {
            let native = ui.input(|input| input.viewport().fullscreen);
            let monitor = ui.input(|input| input.viewport().monitor_size);
            let at_original_size = size.is_some_and(|size| {
                (size - egui::vec2(SIZES[0][0], SIZES[0][1]))
                    .abs()
                    .max_elem()
                    <= 1.
            });
            let outer = ui.input(|input| input.viewport().outer_rect);
            let app = objc2_app_kit::NSApplication::sharedApplication(
                objc2::MainThreadMarker::new().unwrap(),
            );
            let options = app.presentationOptions();
            if self.index == 0 && report.desktop_rect.is_none() {
                report.desktop_rect = outer;
                report.presentation_before = Some(options);
            }
            let bounds_restored = outer
                .zip(report.desktop_rect)
                .is_some_and(|(outer, before)| {
                    (outer.min - before.min).abs().max_elem() <= 1.
                        && (outer.max - before.max).abs().max_elem() <= 1.
                });
            let display_covered = app
                .keyWindow()
                .and_then(|window| {
                    let screen = window.screen()?;
                    let frame = window.frame();
                    let target = screen.frame();
                    Some(
                        (frame.origin.x - target.origin.x).abs() <= 1.
                            && (frame.origin.y - target.origin.y).abs() <= 1.
                            && (frame.size.width - target.size.width).abs() <= 1.
                            && (frame.size.height - target.size.height).abs() <= 1.
                            && window.isOnActiveSpace()
                            && !window
                                .styleMask()
                                .contains(objc2_app_kit::NSWindowStyleMask::FullScreen),
                    )
                })
                .unwrap_or(false);
            let at_fullscreen_size = display_covered
                && native == Some(false)
                && options
                    == (objc2_app_kit::NSApplicationPresentationOptions::AutoHideDock
                        | objc2_app_kit::NSApplicationPresentationOptions::AutoHideMenuBar)
                && outer
                    .zip(size)
                    .is_some_and(|(outer, inner)| (outer.size() - inner).abs().max_elem() <= 1.)
                && size
                    .zip(monitor)
                    .is_some_and(|(size, monitor)| (size - monitor).abs().max_elem() <= 1.);
            let ready = if self.index == 1 {
                at_fullscreen_size && focused
            } else {
                native == Some(false) && at_original_size && bounds_restored && focused
            };
            if self.index < SIZES.len() && ready {
                if report.painted[self.index] {
                    if self.index == 1 {
                        report.fullscreen_entered = true;
                    }
                    if self.index == 2 {
                        report.fullscreen_returned = true;
                        report.presentation_restored = report.presentation_before == Some(options);
                    }
                    self.index += 1;
                    report.sizes = self.index;
                } else {
                    ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                        ui.ctx().content_rect(),
                        PaintAck {
                            report: self.report.clone(),
                            index: self.index,
                        },
                    ));
                }
            }
        }
        if self.mode == Mode::Lifecycle
            && self.index < SIZES.len()
            && size.is_some_and(|s| {
                (s - egui::vec2(SIZES[self.index][0], SIZES[self.index][1]))
                    .abs()
                    .max_elem()
                    <= 1.
            })
        {
            // UI passes can run without a drawable or be discarded for layout. Advance only
            // after the preceding size reached the host's real surface paint pass.
            if report.painted[self.index] {
                self.index += 1;
                report.sizes = self.index;
                if self.index < SIZES.len() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                            SIZES[self.index][0],
                            SIZES[self.index][1],
                        )));
                }
            } else {
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    ui.max_rect(),
                    PaintAck {
                        report: self.report.clone(),
                        index: self.index,
                    },
                ));
            }
        }
        if self.index == SIZES.len() && report.focused {
            report.completed = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}

fn check_result_contract() {
    let mut report = Report::default();
    assert_eq!(
        report.validate(4),
        Err("native host interrupted before completion")
    );
    report.completed = true;
    assert_eq!(report.validate(4), Err("native host did not cleanly exit"));
    report.exited = true;
    report.sizes = SIZES.len();
    assert_eq!(
        report.validate(4),
        Err("native focus/resize forwarding incomplete")
    );
    report.focused = true;
    report.sizes -= 1;
    assert_eq!(
        report.validate(4),
        Err("native focus/resize forwarding incomplete")
    );
    report.sizes = SIZES.len();
    report.painted = [true; SIZES.len()];
    assert_eq!(report.validate(0), Err("native host presented zero frames"));
    report.painted[2] = false;
    assert_eq!(
        report.validate(4),
        Err("native host did not paint and present every requested size")
    );
    report.painted[2] = true;
    assert_eq!(
        report.validate(3),
        Err("native host did not paint and present every requested size")
    );
    assert_eq!(report.validate(4), Ok(()));
    assert_eq!(
        report.validate_fullscreen(4),
        Err("photo-only fullscreen round trip incomplete")
    );
    report.fullscreen_entered = true;
    report.fullscreen_returned = true;
    assert_eq!(
        report.validate_fullscreen(4),
        Err("photo-only fullscreen round trip incomplete")
    );
    report.presentation_restored = true;
    assert_eq!(report.validate_fullscreen(4), Ok(()));
    report.timed_out = true;
    assert_eq!(report.validate(4), Err("native host timed out"));
}

fn main() -> host::Result<()> {
    check_result_contract();
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if !arguments.iter().any(|arg| arg == "--run") {
        println!(
            "Native window tests not requested. Use cargo test --features ui --test native_host -- --run lifecycle"
        );
        return Ok(());
    }
    if !cfg!(target_os = "macos") {
        return Err("native host tests currently require macOS/Metal".into());
    }
    let mode = match arguments.as_slice() {
        [run] if run == "--run" => Mode::Lifecycle,
        [run, case] if run == "--run" => match case.as_str() {
            "lifecycle" => Mode::Lifecycle,
            "fullscreen" => Mode::Fullscreen,
            "editor" => Mode::Editor,
            "early-close" => Mode::EarlyClose,
            "timeout" => Mode::Timeout,
            "process-timeout" => Mode::ProcessTimeout,
            _ => return Err("unknown native host case".into()),
        },
        _ => {
            return Err("usage: native_host --run [lifecycle|fullscreen|editor|early-close|timeout|process-timeout]"
                .into());
        }
    };
    if mode == Mode::Editor && !cfg!(feature = "editor") {
        return Err("native Editor case requires --features editor".into());
    }
    #[cfg(feature = "editor")]
    let editor_fixture = if mode == Mode::Editor {
        let directory = tempfile::tempdir()?;
        // Safety: this harness-free executable has not started threads or the native host.
        // Isolate Editor's default thumbnail cache; no personal preferences are restored.
        unsafe {
            std::env::set_var("HOME", directory.path());
        }
        let path = editor::fixture_project(directory.path())?;
        Some((directory, path))
    } else {
        None
    };
    let report = Arc::new(Mutex::new(Report::default()));
    let outcome = report.clone();
    let (stop, receive) = mpsc::channel();
    let process_deadline = if mode == Mode::ProcessTimeout {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(20)
    };
    let watchdog = std::thread::spawn(move || {
        if matches!(
            receive.recv_timeout(process_deadline),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            eprintln!("native host process deadline exceeded");
            std::process::exit(1);
        }
    });
    let result = if mode == Mode::Editor {
        #[cfg(feature = "editor")]
        {
            let path = editor_fixture.as_ref().unwrap().1.clone();
            let editor_state = std::cell::RefCell::new(None);
            let owner = &editor_state;
            host::run(
                "Antitone native Editor test",
                move |device, queue, _access| {
                    *owner.borrow_mut() =
                        Some(antitone::editor::Editor::new(device, queue, Some(path)));
                    Ok(editor::EditorApp {
                        editor: owner,
                        report: outcome,
                        start: Instant::now(),
                        requested: false,
                        index: 0,
                    })
                },
            )
        }
        #[cfg(not(feature = "editor"))]
        {
            unreachable!("editor feature checked before launch")
        }
    } else {
        host::run(
            "Antitone native host tests",
            move |_device, _queue, _application| {
                Ok(NativeApp {
                    submissions: SubmissionGate::default(),
                    report: outcome,
                    mode,
                    start: Instant::now(),
                    requested: false,
                    index: 0,
                })
            },
        )
    };
    stop.send(()).unwrap();
    watchdog.join().unwrap();
    let presented = result?;
    let report = report.lock().unwrap();
    if mode == Mode::Fullscreen {
        report.validate_fullscreen(presented)?;
    } else {
        report.validate(presented)?;
    }
    let states = if mode == Mode::Fullscreen {
        "fullscreen paint states"
    } else {
        "sizes"
    };
    println!(
        "native host completed: focus, {} {states}, {presented} presented frames, clean close",
        SIZES.len()
    );
    Ok(())
}
