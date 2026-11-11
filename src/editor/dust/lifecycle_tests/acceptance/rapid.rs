use super::*;

#[test]
fn rapid_stroke_batches_publish_every_accepted_repair_with_the_latest_grade_after_leaving_dust() {
    let mut e = coherent_editor();
    e.begin_dust_cleanup();
    let source = e.document.visit.current_source.unwrap();
    let area = e.source_context().unwrap().scan().unwrap().area;
    assert!(
        area.width >= 80 && area.height >= 80,
        "fixture needs room for64 separate masks and valid donor exteriors"
    );
    with_search(e, |app, gate| {
        for batch in 0..8 {
            app.enter(|e, cx| {
                for within in 0..8 {
                    let i = batch * 8 + within;
                    let origin = [
                        area.x + 16 + (i % 8) * (area.width - 40) / 7,
                        area.y + 16 + (i / 8) * (area.height - 40) / 7,
                    ];
                    queue(e, origin, cx);
                }
                assert_eq!(e.pending_dust_strokes().len(), 8);
            });
            for _ in 0..8 {
                gate.next(app)
                    .complete(Ok(SearchOutcome::Candidate([8, 8])));
            }
            settled(app);
            app.read(|e| {
                assert_eq!(
                    e.dust_repairs(source).unwrap().repairs().len(),
                    (batch + 1) as usize * 8
                );
            });
            // Request the next frame without waiting for the previous frame to finish.
            paint(app);
            app.pump();
        }
        let wanted = app.enter(|e, _| {
            let before = capture(e);
            let exposure = e.photo_settings().exposure;
            e.end_tool();
            assert!(!e.dusting());
            e.photo_settings_mut().exposure += 0.5;
            assert_eq!(e.photo_settings().exposure, exposure + 0.5);
            let wanted = capture(e);
            assert_ne!(
                wanted, before,
                "the fixture must request a genuinely new grade"
            );
            wanted
        });
        paint(app);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        app.read(|e| {
            let completed = e
                .document
                .visit
                .working_view
                .completed_capture()
                .unwrap()
                .recipe()
                .unwrap();
            assert_eq!(
                completed.as_ref(),
                &capture(e),
                "an obsolete partial repair list or grade must not replace the latest photograph"
            );
            assert_eq!(completed.as_ref(), &wanted);
            assert_eq!(completed.repairs.len(), 64);
            assert!(
                e.dust_repairs(source)
                    .unwrap()
                    .repairs()
                    .iter()
                    .all(|r| r.offset == [8, 8])
            );
        });
    });
}
