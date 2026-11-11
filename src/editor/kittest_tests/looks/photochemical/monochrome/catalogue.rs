use super::*;
use crate::looks::monochrome::{Negative as BwNegative, Selection as BwSelection};

#[test]
fn bw_negative_and_paper_select_independently_and_other_families_stay_reachable() {
    fixture_editor(|mut h, _| {
        let before = h.read(|e| e.document.project.clone());
        let input = h.read(|e| e.document.project.input);
        let first = BwNegative::from_id("kodak_tmax_100_d76_7m30s").unwrap();
        choose_component(&mut h, "Negative", first.name());
        h.read(|e| {
            let asset = e.document.project.output.film_and_paper.as_ref().unwrap();
            assert_eq!(
                BwSelection::from_origin(asset.origin()),
                Some(BwSelection {
                    negative: Some(first),
                    paper: MonochromePaper::default(),
                })
            );
        });
        open_component(&mut h, "Paper");
        assert!(h.get_by_label("None").accesskit_node().is_disabled());
        h.get_by_label(Paper::default().name()).scroll_to_me();
        h.try_run_realtime().unwrap();
        assert!(
            !h.get_by_label(Paper::default().name())
                .accesskit_node()
                .is_disabled()
        );
        h.key_press(egui::Key::Escape);
        h.step();
        let mut selections = Vec::new();
        for paper in MonochromePaper::all() {
            choose_component(&mut h, "Paper", paper.name());
            for negative in BwNegative::all() {
                choose_component(&mut h, "Negative", negative.name());
                h.read(|e| {
                    assert!(!e.dialogs.is_open());
                    let asset = e.document.project.output.film_and_paper.as_ref().unwrap();
                    let selection = BwSelection {
                        negative: Some(negative),
                        paper,
                    };
                    assert_eq!(BwSelection::from_origin(asset.origin()), Some(selection));
                    assert_eq!(asset.origin(), &selection.origin());
                    assert_eq!(e.document.project.input, input);
                    assert_eq!(e.document.project.output.lut, before.output.lut);
                    assert_eq!(e.document.project.output.grain, before.output.grain);
                    selections.push(selection);
                });
            }
        }
        assert_eq!(
            selections.len(),
            MonochromePaper::all().len() * BwNegative::all().len()
        );
        let persisted = BwSelection {
            negative: BwNegative::from_id("ilford_delta_400_id11_8m_24c"),
            paper: MonochromePaper::from_id("ilford_multigrade_fb_classic_filter_2").unwrap(),
        };
        choose_component(&mut h, "Paper", persisted.paper.name());
        choose_component(&mut h, "Negative", persisted.negative.unwrap().name());
        h.read(|e| {
            let asset = e.document.project.output.film_and_paper.as_ref().unwrap();
            assert_eq!(BwSelection::from_origin(asset.origin()), Some(persisted));
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bw-negative-and-paper.antitone");
        let selected = h.read(|e| e.document.project.output.film_and_paper.clone().unwrap());
        h.enter(|e, cx| e.save_project(path.clone(), crate::project::Save::New, None, cx));
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.read(|e| assert!(!e.dialogs.is_open() && !e.document.dirty()));
        assert_eq!(
            crate::project::Project::load(&path)
                .unwrap()
                .output
                .film_and_paper,
            Some(selected)
        );
        choose_component(&mut h, "Negative", "None");
        h.read(|e| {
            let asset = e.document.project.output.film_and_paper.as_ref().unwrap();
            assert_eq!(
                BwSelection::from_origin(asset.origin()).unwrap().negative,
                None
            );
        });
        choose_component(&mut h, "Paper", "None");
        h.read(|e| assert!(e.document.project.output.film_and_paper.is_none()));
        // A positive colour control rules out a universally disabled catalogue.
        choose_component(&mut h, "Negative", "Kodak Portra 400");
        open_component(&mut h, "Negative");
        h.get_by_label(first.name()).scroll_to_me();
        h.try_run_realtime().unwrap();
        assert!(!h.get_by_label(first.name()).accesskit_node().is_disabled());
        h.key_press(egui::Key::Escape);
        h.step();
    });
}
