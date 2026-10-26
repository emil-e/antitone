use super::*;

#[test]
fn matched_reference_reaches_selected_canvas_and_navigation_at_zero_manual_exposure() {
    let file = crate::exposure_fixture::neutral_dng(1.3);
    editor_harness_with_thumbnails(true, |mut h, capture| {
        h.update(|editor| {
            editor.input = crate::processing::InputSettings::Digital;
            editor.target = OutputTarget::Srgb;
            editor.sync_document();
        });
        h.enter(|editor, cx| editor.add_raws(vec![file.path().to_owned()], cx));
        wait_for(&mut h, |editor| {
            !editor.busy
                && editor.document_pending.is_none()
                && editor.working.support_ready()
                && editor
                    .thumbnails
                    .ready_count(&editor.document, editor.target)
                    == 1
        });
        h.event(egui::Event::PointerGone);
        h.step();
        h.render().unwrap();
        let image = h.read(|editor| {
            assert!(!editor.source);
            assert_eq!(editor.roll.exposure, 0.);
            assert_eq!(editor.adjustment_layer_settings.exposure, 0.);
            let reference = editor
                .document
                .visit
                .ready
                .as_ref()
                .unwrap()
                .developed
                .reference_exposure
                .get()
                .unwrap();
            assert!(matches!(
                reference,
                crate::exposure_reference::ReferenceExposure::Matched(_)
            ));
            assert!((reference.ev() - 1.3).abs() < 0.03, "{reference:?}");
            editor.last_layout.unwrap().0.image.center()
        });
        let card = h
            .get_by_label(file.path().file_name().unwrap().to_str().unwrap())
            .rect()
            .center();
        let linear = crate::rendering::CpuRenderer::new(OutputTarget::Srgb)
            .unwrap()
            .render(&[DVec3::splat(6000. / 65535. * 1.3_f64.exp2())])
            .unwrap()[0];
        let expected = linear
            .to_array()
            .map(|v| OutputTarget::Srgb.encode(f64::from(v)).unwrap());
        let captured = capture.borrow();
        for (name, point) in [("selected canvas", image), ("navigation positive", card)] {
            let actual = captured.pixels
                [point.y.floor() as usize * captured.size[0] as usize + point.x.floor() as usize];
            for c in 0..3 {
                assert!(
                    (f64::from(actual[c]) - expected[c]).abs() < 0.004,
                    "{name}: {actual:?} vs independent reference {expected:?}"
                );
            }
        }
        drop(captured);

        h.get_by_label("Original-resolution reference").click();
        h.step();
        wait_for(&mut h, |editor| {
            assert!(
                editor.original.refinement_error.is_none(),
                "{}",
                editor.status
            );
            editor.working.reference_ready()
        });
        h.render().unwrap();
        let point = h.read(|editor| editor.last_layout.unwrap().0.image.center());
        {
            let pixels = capture.borrow();
            let actual = pixels.pixels
                [point.y.floor() as usize * pixels.size[0] as usize + point.x.floor() as usize];
            for c in 0..3 {
                assert!(
                    (f64::from(actual[c]) - expected[c]).abs() < 0.004,
                    "original reference {actual:?} vs {expected:?}"
                );
            }
        }
        h.get_by_label("Original-resolution reference").click();
        h.step();
        h.enter(|editor, cx| editor.begin_pick(Pick::Wb, cx));
        h.step();
        wait_for(&mut h, |editor| {
            assert!(
                editor.original.refinement_error.is_none(),
                "{}",
                editor.status
            );
            editor.original.settled.is_some()
        });
        h.event(egui::Event::PointerGone);
        h.step();
        h.render().unwrap();
        let point = h.read(|editor| {
            assert_eq!(editor.pick, Some(Pick::Wb));
            editor.last_layout.unwrap().0.image.center()
        });
        let pixels = capture.borrow();
        let actual = pixels.pixels
            [point.y.floor() as usize * pixels.size[0] as usize + point.x.floor() as usize];
        for c in 0..3 {
            assert!(
                (f64::from(actual[c]) - expected[c]).abs() < 0.004,
                "WB precision view {actual:?} vs {expected:?}"
            );
        }
    });
}

#[test]
fn automatic_digital_import_prepares_reused_negative_pixels_before_ready() {
    let first = crate::exposure_fixture::neutral_dng(1.3);
    let second = fixture("patches.dng");
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.add_raws(vec![first.path().to_owned()], cx));
        wait_for(&mut h, |editor| {
            !editor.busy && editor.document_pending.is_none() && editor.working.support_ready()
        });
        let (selected, native, development) = h.read(|editor| {
            assert!(editor.input.is_negative());
            let ready = editor.document.visit.ready.as_ref().unwrap();
            assert!(ready.developed.reference_exposure.get().is_none());
            (
                editor.current_source,
                ready.developed.clone(),
                ready.record.development.clone(),
            )
        });
        h.enter(|editor, cx| editor.add_raws(vec![second], cx));
        wait_for(&mut h, |editor| {
            !editor.busy && editor.document_pending.is_none() && editor.working.support_ready()
        });
        h.render().unwrap();
        h.read(|editor| {
            assert_eq!(editor.input, crate::processing::InputSettings::Digital);
            assert_eq!(editor.current_source, selected);
            let ready = editor.document.visit.ready.as_ref().unwrap();
            assert_eq!(
                ready.record.development, development,
                "the original unity/as-shot basis is unchanged"
            );
            assert!(
                Arc::ptr_eq(&native, &ready.developed),
                "the selected pixels must not be redeveloped"
            );
            let reference = ready
                .developed
                .reference_exposure
                .get()
                .expect("Digital readiness must include preparation");
            assert!((reference.ev() - 1.3).abs() < 0.03, "{reference:?}");
            assert!(!editor.source);
            assert_eq!(editor.roll.exposure, 0.);
            assert_eq!(editor.adjustment_layer_settings.exposure, 0.);
        });
    });
}
