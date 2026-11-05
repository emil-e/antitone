use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[test]
fn pending_material_preparation_preserves_the_independent_lut_and_cancel_keeps_both_accepted_slots()
{
    fixture_editor(|mut h, _| {
        let accepted = constant_asset(0.4);
        h.enter(|e, cx| e.select_look(look::Source::Loaded(accepted.clone()), cx));
        paint_until(&mut h, positive_ready);
        let before = h.read(|e| e.document.project.clone());
        let (send, receive) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let fired = Arc::new(AtomicBool::new(false));
        let prepared = Arc::new(AtomicBool::new(false));
        let observed = prepared.clone();
        h.update(|e| {
            e.look
                .services
                .as_mut()
                .unwrap()
                .observe_prepared(Arc::new(move |valid, stop| {
                    if fired.swap(true, Ordering::AcqRel) {
                        return;
                    }
                    assert!(valid);
                    send.send(stop.clone()).unwrap();
                    observed.store(true, Ordering::Release);
                    let deadline = Instant::now() + Duration::from_secs(90);
                    while !stop.is_cancelled() {
                        assert!(
                            Instant::now() < deadline,
                            "paused selection was not retired"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    let _ = finished.send(());
                }))
        });
        h.enter(|e, cx| {
            e.select_look(
                look::Source::Photochemical(
                    Selection {
                        negative: Negative::from_id("kodak_portra_400"),
                        paper: Paper::default(),
                    }
                    .into(),
                ),
                cx,
            )
        });
        wait_for_with_timeout(&mut h, Duration::from_secs(90), |_| {
            prepared.load(Ordering::Acquire)
        });
        let stop = receive.recv_timeout(Duration::from_secs(5)).unwrap();
        h.read(|e| assert_eq!(e.document.project, before));
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "LUT")
            .scroll_to_me();
        h.run_steps(12);
        assert!(
            h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "LUT")
                .accesskit_node()
                .is_disabled()
        );
        let request = h.read(|e| e.document.look.as_ref().unwrap().activity.clone());
        h.enter(|e, cx| e.select_look(look::Source::Loaded(constant_asset(0.7)), cx));
        h.read(|e| {
            assert_eq!(e.document.look.as_ref().unwrap().activity, request);
            assert_eq!(e.document.project, before);
        });
        h.get_by_label("Cancel look preparation").click();
        h.step();
        paint_until(&mut h, positive_ready);
        assert!(stop.is_cancelled());
        done.recv_timeout(Duration::from_secs(5)).unwrap();
        h.read(|e| assert_eq!(e.document.project, before));
        choose_component(&mut h, "Paper", Paper::default().name());
        h.read(|e| {
            assert_eq!(e.document.project.output.lut.as_ref(), Some(&accepted));
            assert!(e.document.project.output.film_and_paper.is_some());
        });
        choose(&mut h, "None");
        paint_until(&mut h, positive_ready);
        h.read(|e| {
            assert!(e.document.project.output.lut.is_none());
            assert!(e.document.project.output.film_and_paper.is_some());
        });
    });
}
