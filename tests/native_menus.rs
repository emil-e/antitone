//! Explicit AppKit menu/event-loop check. Ordinary invocation never opens a window.
use antitone::platform::{
    SubmissionGate, host,
    menu::{self, Action},
};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSMenu};
use objc2_foundation::{NSPoint, NSString, ns_string};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn main_menu() -> objc2::rc::Retained<NSMenu> {
    NSApplication::sharedApplication(MainThreadMarker::new().unwrap())
        .mainMenu()
        .unwrap()
}

fn submenu(title: &str) -> objc2::rc::Retained<NSMenu> {
    main_menu()
        .itemWithTitle(&NSString::from_str(title))
        .unwrap()
        .submenu()
        .unwrap()
}

fn shortcut(character: &str) {
    let character = NSString::from_str(character);
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown, NSPoint::new(0., 0.), NSEventModifierFlags::Command,
        0., 0, None, &character, &character, false, 0,
    ).unwrap();
    assert!(
        main_menu().performKeyEquivalent(&event),
        "native shortcut was not handled"
    );
}

struct MenuApp {
    submissions: SubmissionGate,
    state: menu::State,
    installed: bool,
    minimizing: Option<objc2::rc::Retained<objc2_app_kit::NSWindow>>,
    stage: usize,
    settled: bool,
    waiting: bool,
    dispatched: Instant,
    completed: Arc<AtomicBool>,
}

impl host::App for MenuApp {
    fn submission_gate(&self) -> &SubmissionGate {
        &self.submissions
    }
    fn menu_state(&self) -> Option<menu::State> {
        Some(self.state.clone())
    }
    fn native_menu_installed(&mut self) {
        self.installed = true;
    }
    fn menu_action(&mut self, action: Action) {
        assert!(self.waiting);
        assert!(
            self.dispatched.elapsed() < Duration::from_millis(500),
            "menu event did not promptly wake the idle host"
        );
        assert_eq!(
            action,
            [Action::Settings, Action::Save, Action::Quit][self.stage]
        );
        self.stage += 1;
        self.waiting = false;
        self.settled = false;
        self.state.enabled.insert(Action::Save);
        self.state.scopes_visible = true;
    }
    fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Native menu integration check");
        assert!(self.installed);
        if self.waiting {
            assert!(
                self.dispatched.elapsed() < Duration::from_secs(2),
                "native menu activation was lost"
            );
            ui.ctx().request_repaint_after(Duration::from_secs(2));
            return;
        }
        if !self.settled {
            self.settled = true;
            ui.ctx().request_repaint();
            return;
        }
        if self.stage == 0
            && let Some(window) = &self.minimizing
        {
            if window.isMiniaturized() {
                self.waiting = true;
                self.dispatched = Instant::now();
                shortcut(",");
            }
            ui.ctx().request_repaint_after(Duration::from_millis(16));
            return;
        }
        match self.stage {
            0 => {
                let root = main_menu();
                let titles: Vec<_> = (0..root.numberOfItems())
                    .map(|i| root.itemAtIndex(i).unwrap().title().to_string())
                    .collect();
                assert_eq!(
                    titles,
                    ["Antitone", "File", "Edit", "View", "Window", "Help"]
                );
                assert!(
                    !submenu("File")
                        .itemWithTitle(ns_string!("Save"))
                        .unwrap()
                        .isEnabled()
                );
                let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
                assert!(
                    app.windowsMenu().is_some()
                        && app.helpMenu().is_some()
                        && app.servicesMenu().is_some()
                );
                let bridged = submenu("Edit");
                assert!(
                    !bridged
                        .itemWithTitle(ns_string!("Copy"))
                        .unwrap()
                        .isEnabled()
                );
                menu::with_native_dialog(|| {
                    let native = submenu("Edit");
                    assert!(native.autoenablesItems());
                    let copy = native.itemWithTitle(ns_string!("Copy")).unwrap();
                    assert_eq!(copy.action(), Some(sel!(copy:)));
                    assert!(copy.target().is_none());
                });
                assert_eq!(
                    submenu("Edit"),
                    bridged,
                    "file-panel Edit did not restore our text bridge"
                );
                let Some(window) = app.keyWindow() else {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                    ui.ctx().request_repaint_after(Duration::from_millis(16));
                    return;
                };
                window.miniaturize(None);
                self.minimizing = Some(window);
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            }
            1 => {
                let window = self.minimizing.as_ref().unwrap();
                // Miniaturization and restoration complete asynchronously in AppKit.
                if window.isMiniaturized() || !window.isVisible() || !window.isKeyWindow() {
                    ui.ctx().request_repaint_after(Duration::from_millis(16));
                    return;
                }
                let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
                assert_eq!(app.keyWindow().as_ref(), Some(window));
                let file = submenu("File");
                assert!(file.itemWithTitle(ns_string!("Save")).unwrap().isEnabled());
                assert_eq!(
                    submenu("View")
                        .itemWithTitle(ns_string!("Show Scopes"))
                        .unwrap()
                        .state(),
                    1
                );
                self.waiting = true;
                self.dispatched = Instant::now();
                file.performActionForItemAtIndex(file.indexOfItemWithTitle(ns_string!("Save")));
            }
            2 => {
                self.waiting = true;
                self.dispatched = Instant::now();
                shortcut("q");
            }
            3 => {
                // Native Quit emits an application action rather than terminating AppKit.
                self.completed.store(true, Ordering::SeqCst);
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => unreachable!(),
        }
        if self.waiting {
            ui.ctx().request_repaint_after(Duration::from_secs(2));
        }
    }
}

fn main() -> host::Result<()> {
    if !std::env::args().any(|arg| arg == "--run") {
        println!("Native menus require explicit --run and a macOS desktop.");
        return Ok(());
    }
    let completed = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog = finished.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(15));
        if !watchdog.load(Ordering::SeqCst) {
            eprintln!("native menu check watchdog expired before host exit");
            std::process::exit(1);
        }
    });
    let report = completed.clone();
    let presented = host::run("Antitone menu check", move |_, _| {
        let mut state = menu::State::default();
        state
            .enabled
            .extend([Action::Settings, Action::Quit, Action::Scopes]);
        Ok(MenuApp {
            submissions: SubmissionGate::default(),
            state,
            installed: false,
            minimizing: None,
            stage: 0,
            settled: false,
            waiting: false,
            dispatched: Instant::now(),
            completed: report,
        })
    })?;
    assert!(completed.load(Ordering::SeqCst) && presented > 0);
    finished.store(true, Ordering::SeqCst);
    println!(
        "Native menus: installation, state updates, shortcuts, activation wakeup, minimized-window restoration, guarded Quit bridge and native-panel Edit restoration passed."
    );
    Ok(())
}
