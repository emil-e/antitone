//! Explicit native host lifecycle check; ordinary Cargo test invocations never open its window.
//! Run with --run [lifecycle|early-close|timeout|process-timeout]. Requires macOS/Metal and a desktop, no RAWs.
//! Checks focus/size forwarding, native resize, submitted frames, clean exit and explicit completion.
//! Early close, timeout, absent completion or zero frames fail. A process watchdog bounds hangs.
//! Does not establish physical gestures, native dialogs, monitor colour or broader-device acceptance.
use antitone::platform::{SubmissionGate, host};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Lifecycle,
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
    fn submission_gate(&self) -> &SubmissionGate {
        &self.submissions
    }
    fn on_exit(&mut self) {
        self.report.lock().unwrap().exited = true;
    }
    fn ui(&mut self, ui: &mut egui::Ui) {
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
            "early-close" => Mode::EarlyClose,
            "timeout" => Mode::Timeout,
            "process-timeout" => Mode::ProcessTimeout,
            _ => return Err("unknown native host case".into()),
        },
        _ => {
            return Err(
                "usage: native_host --run [lifecycle|early-close|timeout|process-timeout]".into(),
            );
        }
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
    let result = host::run("Antitone native host tests", move |_device, _queue| {
        Ok(NativeApp {
            submissions: SubmissionGate::default(),
            report: outcome,
            mode,
            start: Instant::now(),
            requested: false,
            index: 0,
        })
    });
    stop.send(()).unwrap();
    watchdog.join().unwrap();
    let presented = result?;
    report.lock().unwrap().validate(presented)?;
    println!(
        "native host completed: focus, {} sizes, {presented} presented frames, clean close",
        SIZES.len()
    );
    Ok(())
}
