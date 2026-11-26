use std::time::{Duration, Instant};

use muda::{Menu, Submenu};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::pump_events::EventLoopExtPumpEvents;
use winit::platform::windows::EventLoopBuilderExtWindows;
use winit::window::WindowId;

use super::*;
use crate::platform::fullscreen::PhotoFullscreen;

#[derive(Default)]
struct TestWindow {
    menu: Option<Menu>,
    window: Option<Window>,
}

impl ApplicationHandler for TestWindow {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Antitone fullscreen test")
                    .with_inner_size(LogicalSize::new(640, 480)),
            )
            .unwrap();
        let menu = Menu::new();
        menu.append(&Submenu::new("View", true)).unwrap();
        let RawWindowHandle::Win32(handle) = window.window_handle().unwrap().as_raw() else {
            unreachable!();
        };
        // SAFETY: the window is live and this is its event-loop thread.
        unsafe { menu.init_for_hwnd(handle.hwnd.get()) }.unwrap();
        self.menu = Some(menu);
        self.window = Some(window);
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn wait_for(
    event_loop: &mut EventLoop<()>,
    app: &mut TestWindow,
    description: &str,
    ready: impl Fn(&Window) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        event_loop.pump_app_events(Some(Duration::from_millis(10)), app);
        if app.window.as_ref().is_some_and(&ready) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out: {description}");
    }
}

#[test]
#[ignore = "opens a native window on the current Windows desktop"]
fn photo_fullscreen_covers_display_and_restores_window_with_menu() {
    let mut event_loop = EventLoop::builder().with_any_thread(true).build().unwrap();
    let mut app = TestWindow::default();
    wait_for(
        &mut event_loop,
        &mut app,
        "window creation",
        Window::is_decorated,
    );

    let window = app.window.as_ref().unwrap();
    let RawWindowHandle::Win32(handle) = window.window_handle().unwrap().as_raw() else {
        unreachable!();
    };
    let hwnd = handle.hwnd.get() as HWND;
    // SAFETY: the test owns the window and its menu throughout the round trip.
    let menu = unsafe { GetMenu(hwnd) };
    assert!(!menu.is_null());
    let normal = (window.outer_position().unwrap(), window.inner_size());
    let mut fullscreen = PhotoFullscreen::default();

    for maximized in [false, true] {
        app.window.as_ref().unwrap().set_maximized(maximized);
        wait_for(&mut event_loop, &mut app, "initial window mode", |window| {
            window.is_maximized() == maximized
        });
        let window = app.window.as_ref().unwrap();
        let original = (window.outer_position().unwrap(), window.inner_size());
        let monitor = window.current_monitor().unwrap();
        let monitor_origin = monitor.position();
        let monitor_size = monitor.size();

        assert_eq!(fullscreen.update(window, true).unwrap(), Some(true));
        wait_for(
            &mut event_loop,
            &mut app,
            "full-monitor client area",
            |window| {
                window.inner_position().unwrap() == monitor_origin
                    && window.inner_size() == monitor_size
            },
        );
        assert!(unsafe { GetMenu(hwnd) }.is_null());

        let window = app.window.as_ref().unwrap();
        assert_eq!(fullscreen.update(window, false).unwrap(), Some(false));
        wait_for(&mut event_loop, &mut app, "restored window", |window| {
            (window.outer_position().unwrap(), window.inner_size()) == original
                && window.is_maximized() == maximized
        });
        assert_eq!(unsafe { GetMenu(hwnd) }, menu);
    }

    app.window.as_ref().unwrap().set_maximized(false);
    wait_for(
        &mut event_loop,
        &mut app,
        "restored unmaximized placement",
        |window| (window.outer_position().unwrap(), window.inner_size()) == normal,
    );
}
