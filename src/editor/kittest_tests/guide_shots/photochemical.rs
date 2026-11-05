use super::*;
use crate::looks::{
    material::Selection as MaterialSelection,
    monochrome::{
        Negative as MonochromeNegative, Paper as MonochromePaper, Selection as MonochromeSelection,
    },
    spektrafilm::{Negative, Paper, Selection},
};

pub(super) fn capture_demo(harness: &mut Harness<'_, Editor>, directory: &Path, prefix: &str) {
    std::fs::create_dir_all(directory).unwrap();
    tab(harness, "Rendering");
    let input = harness.read(|e| e.document.input);
    let grain = harness.read(|e| e.document.project.output.grain);
    let mut routes = vec![
        ("bypass".to_owned(), None),
        (
            "bw-paper".to_owned(),
            Some(MaterialSelection::Monochrome(
                MonochromePaper::default().into(),
            )),
        ),
        (
            "paper-only".to_owned(),
            Some(
                Selection {
                    negative: None,
                    paper: Paper::default(),
                }
                .into(),
            ),
        ),
        (
            "film-paper".to_owned(),
            Some(
                Selection {
                    negative: Negative::from_id("kodak_portra_400"),
                    paper: Paper::default(),
                }
                .into(),
            ),
        ),
    ];
    for paper in MonochromePaper::all() {
        for negative in std::iter::once(None).chain(MonochromeNegative::all().map(Some)) {
            if negative.is_none() && paper == MonochromePaper::default() {
                continue;
            }
            let name = format!("bw-{}-{}", paper.id(), negative.map_or("none", |n| n.id()));
            routes.push((name, Some(MonochromeSelection { negative, paper }.into())));
        }
    }
    for (name, selection) in routes {
        harness.enter(|e, cx| match selection {
            None => e.clear_film_and_paper(),
            Some(selection) => e.select_look(
                super::super::super::look::Source::Photochemical(selection),
                cx,
            ),
        });
        wait_for_with_timeout(harness, Duration::from_secs(600), |e| {
            !e.interaction_blocked()
        });
        settle_view(harness);
        wait_for_with_timeout(harness, Duration::from_secs(600), |e| {
            e.document
                .thumbnails
                .ready_count(&e.document, e.document.target)
                == e.document.project.order.len()
        });
        harness.read(|e| {
            assert_eq!(e.document.input, input);
            assert_eq!(e.document.project.output.grain, grain);
            assert_eq!(
                e.document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .map(|asset| asset.origin().clone()),
                selection.map(MaterialSelection::origin),
                "requested {name} material was not accepted",
            );
            assert!(e.failure.is_none());
        });
        harness
            .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Paper")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        let name = format!("{prefix}-{name}");
        capture(harness, &name);
        let path = directory.join(format!("{name}.antitone"));
        harness.enter(|e, cx| e.save_project(path.clone(), crate::project::Save::New, None, cx));
        wait_for_with_timeout(harness, Duration::from_secs(600), |e| {
            !e.interaction_blocked() && e.document_pending.is_none()
        });
        let project = harness.read(|e| {
            assert!(e.failure.is_none());
            assert!(!e.document.dirty());
            e.document.state.project.clone()
        });
        assert_eq!(
            crate::project::Project::load(&path).unwrap().output,
            project.output
        );
    }
}

#[test]
#[ignore = "requires an explicitly supplied private project and fresh output directories"]
fn supplied_project_renders_and_saves_the_paper_routes_without_reanalysis() {
    let source = PathBuf::from(
        std::env::var_os("ANTITONE_PAPER_DEMO_PROJECT").expect("private project path"),
    );
    let output =
        PathBuf::from(std::env::var_os("ANTITONE_PHOTOCHEMICAL_DEMO").expect("fresh demo output"));
    assert!(source.is_file());
    editor_harness_with_gpu(true, |mut harness, _, _, _| {
        harness.set_size(egui::vec2(1440., 900.));
        harness.update(host::App::native_menu_installed);
        harness.step();
        open_project(&mut harness, source);
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            !e.interaction_blocked()
                && e.document_pending.is_none()
                && e.document.visit.canvas.ready.is_some()
        });
        harness.read(|e| assert!(e.failure.is_none()));
        settle_view(&mut harness);
        capture_demo(&mut harness, &output, "negative");
    });
}

#[test]
#[ignore = "requires an explicitly supplied private Digital RAW and fresh output directories"]
fn supplied_digital_photograph_renders_and_saves_the_same_component_routes() {
    let source =
        PathBuf::from(std::env::var_os("ANTITONE_DIGITAL_DEMO_SOURCE").expect("Digital RAW path"));
    let output =
        PathBuf::from(std::env::var_os("ANTITONE_PHOTOCHEMICAL_DEMO").expect("fresh demo output"));
    assert!(source.is_file());
    editor_harness_with_gpu(true, |mut harness, _, _, _| {
        harness.set_size(egui::vec2(1440., 900.));
        harness.update(host::App::native_menu_installed);
        harness.step();
        harness.enter(|e, cx| e.apply_input(crate::processing::InputSettings::Digital, cx));
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            !e.interaction_blocked()
        });
        harness.enter(|e, cx| e.add_raws(vec![source], cx));
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            !e.interaction_blocked() && e.document.visit.canvas.ready.is_some()
        });
        settle_view(&mut harness);
        capture_demo(&mut harness, &output, "digital");
        let colour = Selection {
            negative: Negative::from_id("kodak_portra_400"),
            paper: Paper::default(),
        };
        harness.enter(|e, cx| {
            e.select_look(
                super::super::super::look::Source::Photochemical(colour.into()),
                cx,
            )
        });
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            !e.interaction_blocked()
        });
        settle_view(&mut harness);
        harness.read(|e| {
            assert!(e.failure.is_none());
            assert_eq!(
                e.document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .unwrap()
                    .origin(),
                &colour.origin()
            );
        });
        harness
            .get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness
            .get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable")
            .click();
        harness.step();
        settle_view(&mut harness);
        harness.read(|e| assert!(e.document.grain.enabled));
        capture(&mut harness, "digital-film-paper-texture");
    });
}
