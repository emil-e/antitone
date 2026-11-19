use super::*;
use egui_kittest::{Harness, kittest::Queryable};

#[test]
fn compact_failure_is_passive_bounded_and_preserves_other_controls() {
    for width in [800., 260.] {
        let reason = "Preview not updated — showing previous image: not enough graphics memory";
        let mut harness = Harness::builder()
            .with_size(egui::vec2(width, 600.))
            .build_ui_state(
                |ui, clicks: &mut usize| {
                    egui::Panel::bottom("status")
                        .exact_size(crate::editor::ui::ACTIVITY_BAR_HEIGHT)
                        .show(ui, |ui| {
                            ContextualStatus::new(StatusLevel::Error, reason).compact(ui);
                        });
                    if ui.button("Other action").clicked() {
                        *clicks += 1;
                    }
                },
                0,
            );
        harness.run();
        assert!(harness.query_by_label(reason).is_some());
        assert!(harness.query_by_label("Retry").is_none());
        assert!(harness.query_by_label("Details").is_none());
        let response = harness.get_by_label(reason).rect();
        assert!(response.left() >= 0. && response.right() <= width);
        assert!(response.height() <= crate::editor::ui::ACTIVITY_BAR_HEIGHT);
        assert!(response.top() > harness.get_by_label("Other action").rect().bottom());
        harness.get_by_label("Other action").click();
        harness.run();
        assert_eq!(*harness.state(), 1);
        assert!(harness.query_by_label(reason).is_some());
    }
}

#[test]
fn compact_failure_remains_visible_in_a_disabled_parent() {
    let reason = "Preview unavailable: calculation failed";
    let mut harness = Harness::new_ui(|ui| {
        ui.add_enabled_ui(false, |ui| {
            ContextualStatus::new(StatusLevel::Error, reason).compact(ui);
        });
    });
    harness.run();
    assert!(harness.query_by_label(reason).is_some());
    assert!(
        harness
            .query_all_by_role(egui::accesskit::Role::Button)
            .next()
            .is_none()
    );
}
