use super::*;
use crate::gpu::evaluation::{BufferImage, ImageLayout};

#[test]
fn repaired_and_unrepaired_visits_reuse_complete_images_through_the_common_cache() {
    let authored = crate::dng_fixture::Fixture {
        side: 768,
        ramp: true,
        ..Default::default()
    }
    .write();
    for repaired in [false, true] {
        let mut editor =
            crate::editor::tests::authored_fixture(&[authored.path().to_str().unwrap()]);
        editor.set_dmin(Some(DVec3::splat(0.8)));
        editor.set_dmax(Some(DVec3::splat(0.2)));
        let id = editor.document.visit.current_source.unwrap();
        if repaired {
            editor
                .document
                .state
                .project
                .source_mut(id)
                .unwrap()
                .repairs
                .insert(
                    crate::dust_repair::SupportMask::new([90, 30], [2, 2], vec![true; 4]).unwrap(),
                    crate::dust_repair::RepairOrigin::Manual,
                    [-40, 0],
                )
                .unwrap();
        }
        let recipe = Arc::new(
            editor
                .view_recipe(
                    editor.input(),
                    editor.document.project_snapshot().roll,
                    editor.photo_settings(),
                )
                .unwrap(),
        );
        assert_eq!(recipe.repairs.len(), usize::from(repaired));
        let mut frame = FrameRaster::full(recipe.composition).unwrap();
        frame.output = [384, 256];
        let demand = Demand {
            spec: SourceRequest::for_recipe(frame, &recipe).unwrap(),
            pixels: editor.source_context().unwrap(),
            capture: PhotographRequest::Recipe(recipe),
        };
        let services = editor.source_services.as_ref().unwrap();
        services.renderers.set_retention_budget(128 << 20);
        let renderer = services
            .renderers
            .get(&services.device, &services.queue)
            .unwrap();
        let store = renderer.evaluation().materializations();
        store.set_test_pressure(crate::sys::MemoryPressure::Normal);
        editor
            .document
            .visit
            .camera
            .pressure(memory::Pressure::Normal);
        with_application(editor, |app| {
            app.enter(|editor, cx| editor.request_working(demand.clone(), cx));
            app.run_until(Duration::from_secs(15), |editor| {
                editor.document.visit.working_view.support_ready()
            });
            let (owner, images) = app.read(|editor| {
                let frame = editor.document.visit.working_view.frame.as_ref().unwrap();
                let buffers = std::iter::once(&frame.source.pixels)
                    .chain(frame.underlay.as_ref().map(|source| &source.pixels))
                    .chain(frame.processed.iter().map(|processed| &processed.pixels));
                let images: Vec<_> = buffers
                    .map(|buffer| {
                        let BufferImage::Produced(image) = buffer else {
                            panic!("real acquired frames require captured materializations")
                        };
                        (
                            image.identity.clone(),
                            image.descriptor.clone(),
                            Arc::downgrade(&image.image),
                        )
                    })
                    .collect();
                assert_eq!(
                    images.len(),
                    4,
                    "detail and underlay need both neutral and processed values"
                );
                (Arc::downgrade(frame), images)
            });
            app.update(|editor| editor.replace_photo_scope(8));
            assert!(
                owner.upgrade().is_none(),
                "retained frames must not masquerade as cache reuse"
            );
            for (key, layout, weak) in &images {
                let cached = store
                    .find_ready::<ImageLayout>(key, |stored| stored == layout.as_ref())
                    .unwrap();
                assert!(
                    Arc::ptr_eq(&cached.image, &weak.upgrade().unwrap()),
                    "complete acquired calculations must survive only through common optional ownership"
                );
            }
            app.enter(|editor, cx| {
                editor
                    .document
                    .visit
                    .camera
                    .pressure(memory::Pressure::Normal);
                editor.request_working(demand.clone(), cx);
            });
            app.run_until(Duration::from_secs(15), |editor| {
                editor.document.visit.working_view.support_ready()
            });
            app.read(|editor| {
                let frame = editor.document.visit.working_view.frame.as_ref().unwrap();
                let buffers = std::iter::once(&frame.source.pixels)
                    .chain(frame.underlay.as_ref().map(|source| &source.pixels))
                    .chain(frame.processed.iter().map(|processed| &processed.pixels));
                for (buffer, (_, _, previous)) in buffers.zip(&images) {
                    let BufferImage::Produced(image) = buffer else {
                        panic!("returned view lost its materialization")
                    };
                    assert!(Arc::ptr_eq(&image.image, &previous.upgrade().unwrap()));
                }
            });
            store.set_retention_enabled(false);
            assert!(
                images.iter().all(|(_, _, weak)| weak.upgrade().is_some()),
                "live views survive optional eviction"
            );
            app.update(|editor| editor.replace_photo_scope(9));
            assert!(
                images.iter().all(|(_, _, weak)| weak.upgrade().is_none()),
                "no image retention may hide outside the common store"
            );
            assert_eq!(renderer.evaluation().retained_bytes(), 0);
        });
    }
}
