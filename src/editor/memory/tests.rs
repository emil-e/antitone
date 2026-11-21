use super::*;
use crate::{
    application::test_support::with_application,
    gpu::evaluation::{ImageValue, RetainedKey},
};
use std::{sync::Arc, time::Duration};

#[test]
fn pressure_stops_optional_retention_without_changing_or_increasing_limits() {
    let limits = CacheLimits {
        cpu_bytes: 1024 << 20,
        gpu_bytes: 256 << 20,
    };
    let normal = CacheBudgets::new(limits, Pressure::Normal);
    for pressure in [Pressure::Warning, Pressure::Critical] {
        let backoff = CacheBudgets::new(limits, pressure);
        assert_eq!((backoff.sources, backoff.images), (0, 0));
    }
    assert_eq!(CacheBudgets::new(limits, Pressure::Normal), normal);
    assert_eq!(limits.cpu_bytes, 1024 << 20);
}

#[test]
fn pressure_delivery_survives_fullscreen_document_replacement_without_a_paint() {
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let ready = editor.document.visit.canvas.ready.clone().unwrap();
    let key = crate::source::SourceKey::for_load(
        &ready.path,
        ready.file.clone(),
        &ready.record.development,
    );
    let limits = CacheLimits {
        cpu_bytes: 37 << 20,
        gpu_bytes: 1 << 20,
    };
    editor.set_cache_limits(limits);
    let services = editor.source_services.as_ref().unwrap();
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    let device = services.device.clone();
    let store = renderer.evaluation().materializations();
    store.set_test_pressure(Pressure::Normal);
    let buffer_bytes = 64;
    let produce = |id| {
        let device = device.clone();
        pollster::block_on(store.obtain(
            RetainedKey::new(("pressure fixture", id)),
            Arc::new(buffer_bytes),
            buffer_bytes,
            move || async move {
                Ok(ImageValue::new(device.create_buffer(
                    &wgpu::BufferDescriptor {
                        label: None,
                        size: buffer_bytes,
                        usage: wgpu::BufferUsages::STORAGE,
                        mapped_at_creation: false,
                    },
                )))
            },
        ))
        .unwrap()
    };
    let live = produce(0);
    let optional = produce(1);
    let optional_weak = Arc::downgrade(&optional.image);
    drop(optional);
    assert!(optional_weak.upgrade().is_some());
    let live_weak = Arc::downgrade(&live.image);
    let (send, receive) = futures_channel::mpsc::unbounded();
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.memory_watch = cx.spawn(move |cx| pressure_changes(receive, cx));
            let id = editor.document.visit.current_source.unwrap();
            let composition =
                editor.source_composition(editor.document.project.source(id).unwrap());
            let layout = editor.document.visit.canvas.viewport.layout_composition(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600., 400.)),
                composition,
                1.,
            );
            editor.canvas_frame.layout = Some((layout, 1.));
            editor.toggle_fullscreen(&context);
            assert!(editor.fullscreen_requested());
            let previous_cache = editor.document.cache.clone();
            let replacement = editor.document.capture();
            editor.install_project(replacement);
            assert!(!Arc::ptr_eq(&previous_cache, &editor.document.cache));
            assert!(editor.fullscreen_requested());
            assert!(editor.memory_watch.is_some());
            editor.document.cache_ready(&ready);
            assert!(editor.document.cache.charges().retained > 0);
        });
        app.pump();
        let project = app.read(|editor| editor.document.project.clone());
        send.unbounded_send(Pressure::Warning).unwrap();
        app.run_until(Duration::from_secs(2), |editor| {
            editor.memory_pressure == Pressure::Warning
        });
        app.read(|editor| {
            assert_eq!(editor.document.cache.charges().retained, 0);
            assert!(!editor.document.cache.can_retain(1));
            assert_eq!(editor.cache_limits(), limits);
            assert_eq!(editor.document.project, project);
            assert!(editor.fullscreen_requested());
            assert!(
                editor.canvas_frame.displayed.is_none(),
                "the replacement was never painted"
            );
        });
        assert_eq!(renderer.evaluation().cache_policy().generated_bytes, 0);
        assert!(optional_weak.upgrade().is_none());
        assert!(
            live_weak.upgrade().is_some(),
            "pressure cannot revoke an active image"
        );
        assert_eq!(live.as_buffer().unwrap().size(), buffer_bytes);
        assert!(
            app.read(|editor| editor.document.cache.get(&key).is_some()),
            "active source inputs remain available at zero retention"
        );

        send.unbounded_send(Pressure::Normal).unwrap();
        app.run_until(Duration::from_secs(2), |editor| {
            editor.memory_pressure == Pressure::Normal
        });
        app.read(|editor| {
            assert!(editor.document.cache.get(&key).is_some());
            assert!(editor.document.cache.charges().retained > 0);
            assert_eq!(editor.cache_limits(), limits);
        });
        assert_eq!(
            renderer.evaluation().cache_policy().generated_bytes,
            limits.gpu_bytes
        );
        drop(produce(1));
        assert!(
            store
                .find_ready::<u64>(&RetainedKey::new(("pressure fixture", 1)), |_| true)
                .is_some()
        );

        send.unbounded_send(Pressure::Critical).unwrap();
        app.run_until(Duration::from_secs(2), |editor| {
            editor.memory_pressure == Pressure::Critical
        });
        app.read(|editor| {
            assert_eq!(editor.document.cache.charges().retained, 0);
            assert_eq!(editor.cache_limits(), limits);
            assert_eq!(editor.document.project, project);
            assert!(editor.fullscreen_requested());
        });
        assert_eq!(renderer.evaluation().retained_bytes(), 0);
        assert!(live_weak.upgrade().is_some());
        app.close_execution();
        assert!(
            send.unbounded_send(Pressure::Normal).is_err(),
            "retiring the application drops its pressure receiver"
        );
    });
}
