use super::*;
use crate::{
    application::test_support::with_application,
    looks::spektrafilm::{Negative, Paper},
};
use std::time::{Duration, Instant};

#[test]
fn pending_component_edits_capture_one_pair_and_reject_paper_bypass_with_a_negative() {
    let (e, scheduler) = super::tests::editor();
    with_application(e, |app| {
        let negative = Negative::from_id("kodak_portra_400").unwrap();
        let paper = Paper::from_id("kodak_2383").unwrap();
        app.enter(|e, cx| {
            e.select_negative(Some(negative.into()), cx);
            assert_eq!(
                e.photochemical_selection(),
                Some(Selection {
                    negative: Some(negative),
                    paper: Paper::default()
                })
            );
            e.select_paper(Some(paper.into()), cx);
            assert_eq!(
                e.photochemical_selection(),
                Some(Selection {
                    negative: Some(negative),
                    paper
                })
            );
            assert!(e.document.project.output.film_and_paper.is_none());
        });
        app.run_until(Duration::from_secs(90), |e| e.document.look.is_none());
        let before = app.read(|e| e.document.state.project.clone());
        app.read(|e| {
            assert_eq!(
                e.photochemical_selection(),
                Some(Selection {
                    negative: Some(negative),
                    paper
                })
            )
        });
        app.enter(|e, cx| e.select_paper(None, cx));
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(e.document.look.is_none());
        });
        app.enter(|e, cx| e.select_negative(None, cx));
        app.run_until(Duration::from_secs(90), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(
                e.photochemical_selection(),
                Some(Selection {
                    negative: None,
                    paper
                })
            )
        });
        app.enter(|e, cx| e.select_paper(None, cx));
        app.read(|e| assert!(e.document.project.output.film_and_paper.is_none()));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn unsupported_material_requests_preserve_pending_and_accepted_choices_until_confirmed() {
    use crate::looks::monochrome;
    let (e, scheduler) = super::tests::editor();
    with_application(e, |app| {
        let imported = super::tests::asset(0.4);
        app.enter(|e, cx| e.select_look(Source::Loaded(imported.clone()), cx));
        app.run_until(Duration::from_secs(30), |e| e.document.look.is_none());
        let paper = monochrome::Paper::default();
        app.enter(|e, cx| {
            e.select_paper(Some(material::Paper::Monochrome(paper)), cx);
            let pending = e.document.look.as_ref().unwrap().activity;
            assert_eq!(
                e.material_selection(),
                Some(MaterialSelection::Monochrome(paper.into()))
            );
            assert_eq!(e.photochemical_selection(), None);
            e.select_negative(Negative::from_id("kodak_portra_400").map(Into::into), cx);
            assert_eq!(e.document.look.as_ref().unwrap().activity, pending);
            assert!(e.document.material_switch.is_some());
            e.resolve_material_switch(false, cx);
            assert_eq!(e.document.look.as_ref().unwrap().activity, pending);
        });
        app.run_until(Duration::from_secs(90), |e| e.document.look.is_none());
        let accepted = app.read(|e| e.document.project.clone());
        app.enter(|e, cx| {
            e.select_negative(Negative::from_id("kodak_portra_400").map(Into::into), cx)
        });
        app.read(|e| {
            assert!(e.document.look.is_none());
            assert_eq!(e.document.project, accepted);
            assert_eq!(e.document.project.output.lut.as_ref(), Some(&imported));
            assert!(e.document.material_switch.is_some());
        });
        app.enter(|e, cx| e.resolve_material_switch(false, cx));
        app.enter(|e, cx| e.select_paper(Some(Paper::default().into()), cx));
        app.run_until(Duration::from_secs(90), |e| e.document.look.is_none());
        app.enter(|e, cx| {
            e.select_negative(Negative::from_id("kodak_portra_400").map(Into::into), cx)
        });
        app.run_until(Duration::from_secs(90), |e| e.document.look.is_none());
        let colour = app.read(|e| e.document.project.clone());
        app.enter(|e, cx| e.select_paper(Some(material::Paper::Monochrome(paper)), cx));
        app.read(|e| {
            assert!(e.document.look.is_none());
            assert_eq!(e.document.project, colour);
            assert!(e.document.material_switch.is_some());
        });
        app.enter(|e, cx| e.resolve_material_switch(false, cx));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn on_demand_native_bakes_cache_without_the_gpu_inverse_or_upstream_runtime() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let limits = adapter.limits();
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::at(directory.path().into());
        let cancel = AtomicBool::new(false);
        for negative in [None, Negative::from_id("kodak_ektar_100")] {
            let selection = Selection {
                negative,
                paper: Paper::default(),
            };
            let start = Instant::now();
            let first = prepare_asset(
                Source::Photochemical(selection.into()),
                &cancel,
                &limits,
                Some(&cache),
            )
            .unwrap()
            .unwrap();
            let cold = start.elapsed();
            let start = Instant::now();
            let second = prepare_asset(
                Source::Photochemical(selection.into()),
                &cancel,
                &limits,
                Some(&cache),
            )
            .unwrap()
            .unwrap();
            let warm = start.elapsed();
            assert_eq!(first, second);
            assert_eq!(first.lut().size(), DEFAULT_GRID);
            assert_eq!(first.origin(), &selection.origin());
            println!(
                "{} {}³ coldgenerate/cache {cold:?}, warmload {warm:?}",
                selection.name(),
                DEFAULT_GRID
            );
        }
        cancel.store(true, Ordering::Relaxed);
        assert!(
            prepare_asset(
                Source::Photochemical(
                    Selection {
                        negative: None,
                        paper: Paper::default()
                    }
                    .into()
                ),
                &cancel,
                &limits,
                Some(&cache)
            )
            .unwrap()
            .is_none()
        );
    });
}
