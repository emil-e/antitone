use super::*;
use crate::looks::monochrome::{Negative as BwNegative, Selection as BwSelection};

fn capture_switch(h: &mut Harness<'_, Editor>, name: &str) {
    if let Some(folder) = std::env::var_os("ANTITONE_UI_CAPTURE") {
        let folder = PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        h.render().unwrap().save(folder.join(name)).unwrap();
    }
}

fn confirm_switch(h: &mut Harness<'_, Editor>) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Switch materials")
        .click();
    h.step();
    paint_until(h, positive_ready);
    h.read(|e| assert!(e.document.material_switch.is_none() && e.failure.is_none()));
}

#[test]
fn negative_family_switch_confirms_default_paper_as_one_undoable_edit_and_cancel_keeps_both_slots()
{
    fixture_editor(|mut h, _| {
        let imported = constant_asset(0.4);
        h.enter(|e, cx| e.select_look(crate::editor::look::Source::Loaded(imported.clone()), cx));
        paint_until(&mut h, positive_ready);
        let negative = BwNegative::from_id("kodak_tmax_100_d76_7m30s").unwrap();
        choose_component(&mut h, "Negative", negative.name());
        let before = h.read(|e| e.document.project.output.clone());
        choose_component(&mut h, "Negative", "Kodak Portra 400");
        h.read(|e| {
            assert!(e.document.material_switch.is_some());
            assert!(e.document.look.is_none());
            assert_eq!(e.document.project.output, before);
            assert!(!e.history_available());
            assert!(!e.tool_start_available());
            let menu = e.application_menu_state();
            for action in [
                crate::platform::menu::Action::Save,
                crate::platform::menu::Action::New,
                crate::platform::menu::Action::Undo,
            ] {
                assert!(!menu.enabled.contains(&action));
            }
        });
        capture_switch(&mut h, "negative-family-switch.png");
        let (selected, other) = h.read(|e| {
            assert!(e.document.project.order.len() >= 2);
            (
                e.document.selected,
                (e.document.selected + 1) % e.document.project.order.len(),
            )
        });
        h.enter(|e, cx| e.select_roll_photo(other, egui::Modifiers::NONE, cx));
        h.read(|e| assert_eq!(e.document.selected, selected));
        h.key_press(egui::Key::Escape);
        h.step();
        h.read(|e| {
            assert!(e.document.material_switch.is_none() && e.document.look.is_none());
            assert_eq!(e.document.project.output, before);
        });
        choose_component(&mut h, "Negative", "Kodak Portra 400");
        confirm_switch(&mut h);
        let after = h.read(|e| {
            let output = &e.document.project.output;
            assert_eq!(output.lut.as_ref(), Some(&imported));
            assert_eq!(
                Selection::from_origin(output.film_and_paper.as_ref().unwrap().origin()),
                Some(Selection {
                    negative: Negative::from_id("kodak_portra_400"),
                    paper: Paper::default()
                })
            );
            output.clone()
        });
        h.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Undo, cx));
        paint_until(&mut h, positive_ready);
        h.read(|e| assert_eq!(e.document.project.output, before));
        h.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Redo, cx));
        paint_until(&mut h, positive_ready);
        h.read(|e| assert_eq!(e.document.project.output, after));
        // Exercise the reverse direction through the same negative selector.
        choose_component(&mut h, "Negative", negative.name());
        confirm_switch(&mut h);
        h.read(|e| {
            assert_eq!(
                BwSelection::from_origin(
                    e.document
                        .project
                        .output
                        .film_and_paper
                        .as_ref()
                        .unwrap()
                        .origin()
                ),
                Some(BwSelection {
                    negative: Some(negative),
                    paper: MonochromePaper::default()
                })
            );
            assert_eq!(e.document.project.output.lut.as_ref(), Some(&imported));
        });
    });
}

#[test]
fn paper_family_switch_clears_the_negative_only_after_confirmation_and_keeps_the_requested_paper() {
    fixture_editor(|mut h, _| {
        choose_component(&mut h, "Negative", "Kodak Portra 400");
        let before = h.read(|e| e.document.project.output.clone());
        let bw_paper = MonochromePaper::from_id("foma_fomabrom_variant_glossy_filter_2").unwrap();
        choose_component(&mut h, "Paper", bw_paper.name());
        h.read(|e| {
            assert!(e.document.material_switch.is_some());
            assert_eq!(e.document.project.output, before);
        });
        capture_switch(&mut h, "paper-family-switch.png");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
            .click();
        h.step();
        h.read(|e| {
            assert!(e.document.material_switch.is_none());
            assert_eq!(e.document.project.output, before);
        });
        choose_component(&mut h, "Paper", bw_paper.name());
        confirm_switch(&mut h);
        h.read(|e| {
            assert_eq!(
                BwSelection::from_origin(
                    e.document
                        .project
                        .output
                        .film_and_paper
                        .as_ref()
                        .unwrap()
                        .origin()
                ),
                Some(bw_paper.into())
            )
        });
        let negative = BwNegative::from_id("kodak_trix_400_d76_8m").unwrap();
        choose_component(&mut h, "Negative", negative.name());
        let colour_paper = Paper::from_id("kodak_2383").unwrap();
        choose_component(&mut h, "Paper", colour_paper.name());
        confirm_switch(&mut h);
        h.read(|e| {
            assert_eq!(
                Selection::from_origin(
                    e.document
                        .project
                        .output
                        .film_and_paper
                        .as_ref()
                        .unwrap()
                        .origin()
                ),
                Some(Selection {
                    negative: None,
                    paper: colour_paper
                })
            );
        });
    });
}
