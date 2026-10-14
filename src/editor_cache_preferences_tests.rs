use super::*;
use cache_limits::{CacheLimits, MIB};

#[test]
fn restored_limits_control_caches_without_changing_project_or_recomputing_defaults() {
    let (mut editor, _send, _receiver) = tests::fixture();
    let project = serde_json::to_value(&editor.document.project).unwrap();
    let limits = CacheLimits {
        cpu_bytes: 37 * MIB,
        gpu_bytes: 0,
    };
    let mut preferences = preferences::Preferences::default();
    preferences.cache_limits = Some(limits);
    editor.apply_preferences(preferences.clone());
    assert_eq!(editor.thumbnails.limits, limits);
    // Repeated repaint/pressure polling cannot select another hardware default.
    for _ in 0..4 {
        editor.thumbnails.update_memory();
    }
    assert_eq!(editor.thumbnails.limits, limits);
    let (mut reopened, _send, _receiver) = tests::fixture();
    reopened.apply_preferences(preferences);
    assert_eq!(reopened.thumbnails.limits, limits);
    assert_eq!(
        serde_json::to_value(&editor.document.project).unwrap(),
        project
    );
}

#[test]
fn cache_controls_are_named_and_reset_persists_without_changing_the_project() {
    let (mut editor, _send, _receiver) = tests::fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    editor.preference_store = Some(preferences::Store::open(path.clone()).unwrap().0);
    let limits = CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 0,
    };
    let mut preferences = preferences::Preferences::default();
    preferences.cache_limits = Some(limits);
    editor.apply_preferences(preferences);
    let project = serde_json::to_value(&editor.document.project).unwrap();
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut frame = |events, enabled| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500., 260.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                ui.add_enabled_ui(enabled, |ui| editor.cache_preferences(ui));
            },
        );
        output.textures_delta.clear();
        output
    };
    let first = frame(vec![], true);
    let nodes = &first
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes;
    for label in ["CPU cache", "GPU cache", "Reset cache limits"] {
        assert!(nodes.iter().any(|(_, node)| node.label() == Some(label)));
    }
    let cpu = nodes
        .iter()
        .find(|(_, node)| node.label() == Some("CPU cache") && node.numeric_value().is_some())
        .unwrap();
    assert_eq!(cpu.1.numeric_value(), Some(0.));
    let cpu = cpu.1.bounds().unwrap();
    let cpu = egui::pos2(
        ((cpu.x0 + cpu.x1) / 2.) as f32,
        ((cpu.y0 + cpu.y1) / 2.) as f32,
    );
    for pressed in [true, false] {
        frame(
            vec![
                egui::Event::PointerMoved(cpu),
                egui::Event::PointerButton {
                    pos: cpu,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
    }
    frame(vec![], true);
    frame(vec![egui::Event::Text("37".into())], true);
    assert!(!path.exists());
    frame(
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
    );
    assert_eq!(
        preferences::Store::open(path.clone())
            .unwrap()
            .1
            .cache_limits,
        Some(CacheLimits {
            cpu_bytes: 37 * MIB,
            gpu_bytes: 0
        })
    );
    let before_disabled = std::fs::read(&path).unwrap();
    let reset = nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Reset cache limits"))
        .unwrap()
        .1
        .bounds()
        .unwrap();
    let point = egui::pos2(
        ((reset.x0 + reset.x1) / 2.) as f32,
        ((reset.y0 + reset.y1) / 2.) as f32,
    );
    let pointer = |pressed| {
        vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    frame(pointer(true), false);
    frame(pointer(false), false);
    assert_eq!(std::fs::read(&path).unwrap(), before_disabled);
    // egui hit-tests widgets registered by the previous frame.
    frame(vec![], true);
    frame(pointer(true), true);
    frame(pointer(false), true);
    drop(frame);
    assert_eq!(editor.thumbnails.limits, editor.thumbnails.default_limits());
    assert_ne!(editor.thumbnails.limits, limits);
    assert_eq!(
        preferences::Store::open(path).unwrap().1.cache_limits,
        Some(editor.thumbnails.limits)
    );
    assert_eq!(
        serde_json::to_value(&editor.document.project).unwrap(),
        project
    );
}
