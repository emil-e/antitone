use super::*;
use crate::dust_repair::{RepairOrigin, SupportMask, nearest_offset};
use crate::photographic::parameters::ProcessingParameters;
use crate::photographic::test_inputs::AuthoredPhotograph;

#[cfg(feature = "ocio-reference")]
mod pixel_tests;

fn gpu() -> (wgpu::Device, crate::gpu::submission::Queue) {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap()
    })
}

fn document() -> crate::editor::document_scope::DocumentScope {
    let mut document = Document::default();
    document
        .import(&[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng")])
        .unwrap();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    document.into()
}

fn complete(
    thumbnails: &mut Thumbnails,
    document: &Document,
    key: ImageKey,
) -> Result<Arc<Product>, String> {
    let id = key.source.id;
    thumbnails.processed.protect_images(vec![key.clone()]);
    let job = tests::service_job(
        thumbnails,
        key.clone(),
        document.source_path(id).unwrap(),
        document.project.source(id).unwrap().clone(),
        None,
        true,
    );
    tests::complete_service_request(thumbnails, document, job);
    thumbnails
        .processed
        .cache
        .get(&key)
        .expect("current result")
        .clone()
}

#[test]
fn authored_positive_cards_keep_grade_reuse_and_freshness() {
    let (device, queue) = gpu();
    let renderers = Arc::new(crate::editor::display::DisplayFixture::for_device(&device));
    let mut thumbnails = Thumbnails::with_disk(device, queue, renderers, None);
    let mut document = document();
    let id = document.project.order[0];
    let source = document.project.source(id).unwrap().clone();
    let scan = DevelopedSource::open(
        &document.source_path(id).unwrap(),
        document.project.acquisition_wb(),
    )
    .unwrap();
    let proxy = Arc::new(Proxy::from_source(&scan).unwrap());
    thumbnails = thumbnails.with_sources(document.cache.clone());
    thumbnails.seed(
        &source,
        FileKey::new(&document.source_path(id).unwrap()),
        proxy,
    );
    drop(scan);
    let plain_key = tests::key(&document, id);
    let started = std::time::Instant::now();
    let plain = complete(&mut thumbnails, &document, plain_key.clone()).unwrap();
    println!(
        "positive_thumbnail_proxy_seconds={:.6}",
        started.elapsed().as_secs_f64()
    );
    assert!(
        plain.positive.is_some(),
        "the cached camera proxy must become a graded positive without original reconstruction"
    );
    assert!(plain_key.current_recipe(&document));
    let neutral = plain.positive.as_ref().unwrap().product().unwrap().clone();
    thumbnails.processed.protect_images(vec![plain_key.clone()]);
    thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 0,
    });
    document
        .project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .exposure = 0.5;
    let mut layers = crate::project::AdjustmentLayers::default();
    layers
        .add(
            "Linear".into(),
            crate::adjustment_layers::AdjustmentLayer {
                mask: crate::adjustment_layers::AdjustmentLayerCoverage::Linear {
                    start: crate::geometry::SourcePoint([0., 0.]),
                    end: crate::geometry::SourcePoint([256., 192.]),
                },
                photo: AdjustmentLayerSettings {
                    exposure: 0.75,
                    ..Default::default()
                },
                enabled: true,
                invert: false,
                strength: 1.,
            },
        )
        .unwrap();
    document.project.source_mut(id).unwrap().adjustment_layers = Some(layers);
    let mut graded_key = tests::key(&document, id);
    graded_key.target = OutputTarget::Srgb;
    let graded = complete(&mut thumbnails, &document, graded_key.clone()).unwrap();
    assert!(
        crate::photographic::graph::authoring::shares_resident_camera(
            &neutral.definition,
            &graded
                .positive
                .as_ref()
                .unwrap()
                .product()
                .unwrap()
                .definition
        ),
        "grade/target changes must reuse the physical camera proxy"
    );
    assert!(!Arc::ptr_eq(&plain.image, &graded.image));
    assert!(
        graded_key.estimated_product_bytes().unwrap()
            >= graded.byte_len() + graded_key.retained_bytes()
    );
    thumbnails
        .processed
        .protect_images(vec![graded_key.clone()]);
    assert!(
        !thumbnails.processed.cache.contains(&plain_key),
        "replacement completion must retire the carried old card at zero optional retention"
    );
    document
        .project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .exposure = 1.;
    let failed_key = tests::key(&document, id);
    thumbnails
        .processed
        .protect_images(vec![failed_key.clone()]);
    thumbnails
        .processed
        .cache
        .insert(failed_key.clone(), Err("grade range rejection".into()));
    thumbnails.processed.protect_images(vec![failed_key]);
    assert!(
        thumbnails.processed.cache.contains(&graded_key),
        "a failed downstream grade must not discard compatible visible neutral content"
    );
    document
        .project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .exposure = 0.25;
    let readmitted = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    assert!(
        crate::photographic::graph::authoring::shares_resident_camera(
            &neutral.definition,
            &readmitted
                .positive
                .as_ref()
                .unwrap()
                .product()
                .unwrap()
                .definition
        )
    );

    assert!(
        !graded_key.current_recipe(&document),
        "a changed grade must still reject stale results"
    );
}

#[test]
fn cached_proxy_finishes_repair_bearing_navigation_cards_without_a_developable_raw() {
    let (device, queue) = gpu();
    let mut thumbnails = Thumbnails::with_disk(
        device.clone(),
        queue,
        Arc::new(crate::editor::display::DisplayFixture::for_device(&device)),
        None,
    );
    // This document has locator-only files, not decodable RAWs: the published
    // camera proxy is the only available image source.
    let mut document: crate::editor::document_scope::DocumentScope = tests::fixture().into();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    let id = document.project.order[0];
    thumbnails = thumbnails.with_sources(document.cache.clone());
    thumbnails.seed(
        document.project.source(id).unwrap(),
        FileKey::new(&document.source_path(id).unwrap()),
        Arc::new(Proxy {
            identity: None,
            width: 8,
            height: 6,
            pixels: vec![[5000., 7000., 9000., 0.]; 48].into(),
            matrix: glam::DMat3::IDENTITY,
        }),
    );
    let plain = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    assert!(plain.positive.is_some());
    document
        .project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .exposure = 0.5;
    let graded = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    assert!(
        crate::photographic::graph::authoring::shares_resident_camera(
            &plain
                .positive
                .as_ref()
                .unwrap()
                .product()
                .unwrap()
                .definition,
            &graded
                .positive
                .as_ref()
                .unwrap()
                .product()
                .unwrap()
                .definition
        ),
        "grading must reuse the small camera buffer, not request originals"
    );
    document
        .project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([120, 60], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            [-64, 0],
        )
        .unwrap();
    thumbnails.processed.cache.retain(|_, _| false);
    let card = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    assert!(
        card.positive.is_some(),
        "accepted repairs must not prevent cached navigation feedback"
    );
}

#[test]
fn thumbnail_looks_reuse_proxy_support_even_when_the_document_has_dust() {
    use crate::looks::{LookAsset, LookOrigin, Lut3D};
    let (device, queue) = gpu();
    let mut thumbnails = Thumbnails::with_disk(
        device.clone(),
        queue,
        Arc::new(crate::editor::display::DisplayFixture::for_device(&device)),
        None,
    );
    let mut document = document();
    let id = document.project.order[0];
    document
        .project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([120, 60], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            [-64, 0],
        )
        .unwrap();
    let plain = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    let neutral = plain.positive.as_ref().unwrap().product().unwrap().clone();
    thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 0,
    });
    let lut = Arc::new(Lut3D::new(2, [-0.5; 3], [1.1; 3], vec![[0.4, 0.3, 0.2]; 8]).unwrap());
    document.project.output.lut = Some(Arc::new(
        LookAsset::new("Card look".into(), LookOrigin::Imported, lut.clone()).unwrap(),
    ));
    let key = tests::key(&document, id);
    let looked = complete(&mut thumbnails, &document, key.clone()).unwrap();
    let view = &looked.positive.as_ref().unwrap();
    assert_eq!(view.processing().look_id(), Some(lut.id()));
    assert!(
        crate::photographic::graph::authoring::shares_resident_camera(
            &neutral.definition,
            &looked
                .positive
                .as_ref()
                .unwrap()
                .product()
                .unwrap()
                .definition
        )
    );
    #[cfg(feature = "ui-tests")]
    assert!(view.same_source_as(plain.positive.as_ref().unwrap()));
    assert!(looked.byte_len() > plain.byte_len());
    assert!(key.estimated_product_bytes().unwrap() >= key.retained_bytes() + looked.byte_len());
    document.project.output.lut = None;
    assert!(!key.current_recipe(&document));
    let restored = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    assert!(
        crate::photographic::graph::authoring::shares_resident_camera(
            &neutral.definition,
            &restored
                .positive
                .as_ref()
                .unwrap()
                .product()
                .unwrap()
                .definition
        )
    );
    assert_eq!(
        restored.positive.as_ref().unwrap().processing().look_id(),
        None
    );
}

#[test]
fn grading_keeps_the_previous_card_visible_and_replacement_does_not_accumulate_versions() {
    let (device, queue) = gpu();
    let presentation = pollster::block_on(Presentation::new(&device)).unwrap();
    let mut thumbnails = Thumbnails::with_disk(
        device.clone(),
        queue,
        Arc::new(crate::editor::display::DisplayFixture::for_device(&device)),
        None,
    );
    let document = document();
    let id = document.project.order[0];
    let initial_key = tests::key(&document, id);
    complete(&mut thumbnails, &document, initial_key).unwrap();
    let Some(Services::Production(worker)) = thumbnails.processed.services.clone() else {
        panic!("fixture requires the production thumbnail worker");
    };
    let (commands, requests) = mpsc::channel();
    thumbnails.processed.services = Some(Services::Controlled(commands));
    let mut editor = Editor::state();
    editor.presentation = Ok(presentation);
    editor.document = document;
    editor.document.thumbnails = thumbnails.with_sources(editor.document.cache.clone());
    editor.load_input(editor.document.state.project.input);
    editor.document.visit.current_source = Some(id);
    let context = egui::Context::default();
    context.enable_accesskit();
    crate::application::test_support::with_application(editor, |app| {
        let frame = |app: &mut crate::application::test_support::TestApplication<'_, Editor>| {
            let mut output = app.enter(|editor, cx| {
                context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100., 800.),
                        )),
                        ..Default::default()
                    },
                    |ui| editor.filmstrip(ui, cx),
                )
            });
            output.textures_delta.clear();
            output
        };
        let gamma = app.read(|e| e.document.state.project.input.gamma());
        for (index, (exposure, gamma)) in [(0.25, gamma), (0.5, gamma), (0.75, gamma), (1., 1.2)]
            .into_iter()
            .enumerate()
        {
            app.update(|editor| {
                if index == 3 {
                    editor.set_cache_limits(cache_limits::CacheLimits {
                        cpu_bytes: 0,
                        gpu_bytes: 0,
                    });
                }
                editor.document.state.project.set_gamma(gamma);
                editor.set_gamma(gamma);
                editor
                    .document
                    .project
                    .source_mut(id)
                    .unwrap()
                    .adjustment_layers
                    .as_mut()
                    .unwrap()
                    .layers[0]
                    .adjustment
                    .photo
                    .exposure = exposure;
                editor.photo_settings_mut().exposure = exposure;
            });
            let mut output = frame(app);
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| matches!(shape.shape, egui::Shape::Callback(_))),
                "grading must not replace the existing image with a blank/JPEG"
            );
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.job.text=="…")));
            assert!(
                output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some("Photo updating")),
                "previous pixels must not claim to be a current grade"
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let submission = loop {
                app.pump();
                if let Ok(submission) = requests.try_recv() {
                    break submission;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "thumbnail never refreshed"
                );
                let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
                std::thread::sleep(delay.min(Duration::from_millis(10)));
                output = frame(app);
            };
            let provider = app.read(|editor| editor.document.thumbnails.acquisition().unwrap());
            let outcome =
                pollster::block_on(worker.clone().process(submission.job, provider)).unwrap();
            assert!(submission.reply.send(outcome).is_ok());
            let key = app.read(|editor| tests::key(&editor.document, id));
            app.run_until(Duration::from_secs(10), |editor| {
                editor.document.thumbnails.processed.cache.contains(&key)
                    && !editor.document.thumbnails.processed.work.pending()
            });
            app.read(|editor| {
                assert_eq!(
                    editor.document.thumbnails.processed.cache.entries.len(),
                    1,
                    "superseded grades must not compete with other roll photos for retention"
                )
            });
            let output = frame(app);
            assert!(
                !output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some("Photo updating"))
            );
        }
        app.update(|editor| {
            editor
                .document
                .project
                .source_mut(id)
                .unwrap()
                .adjustment_layers
                .as_mut()
                .unwrap()
                .layers[0]
                .adjustment
                .photo
                .exposure = 1.25;
            let key = tests::key(&editor.document, id);
            editor
                .document
                .thumbnails
                .processed
                .protect_images(vec![key.clone()]);
            editor
                .document
                .thumbnails
                .processed
                .cache
                .insert(key, Err("render rejected".into()));
        });
        let output = frame(app);
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Callback(_))),
            "an authoritative error must not be hidden by old pixels"
        );
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.job.text=="!")));
    });
}

#[test]
fn replacing_a_grade_preserves_other_photos_in_the_roll_cache() {
    let (device, queue) = gpu();
    let mut thumbnails = Thumbnails::with_disk(
        device.clone(),
        queue,
        Arc::new(crate::editor::display::DisplayFixture::for_device(&device)),
        None,
    );
    let mut document = document();
    document
        .import(&[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/preview.dng")])
        .unwrap();
    let first = document.project.order[0];
    let other = document.project.order[1];
    let first_key = tests::key(&document, first);
    let other_key = tests::key(&document, other);
    complete(&mut thumbnails, &document, first_key.clone()).unwrap();
    let other_product = complete(&mut thumbnails, &document, other_key.clone()).unwrap();
    document
        .project
        .source_mut(first)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .exposure = 0.5;
    let replacement = tests::key(&document, first);
    complete(&mut thumbnails, &document, replacement.clone()).unwrap();
    assert!(!thumbnails.processed.cache.contains(&first_key));
    assert!(thumbnails.processed.cache.contains(&replacement));
    assert!(
        Arc::ptr_eq(
            thumbnails
                .processed
                .cache
                .get(&other_key)
                .unwrap()
                .as_ref()
                .unwrap(),
            &other_product
        ),
        "replacing one photo's grade must preserve another photo's ready pixels"
    );
}

#[test]
fn previous_card_presentation_requires_matching_source_framing_and_target() {
    let mut document = document();
    let id = document.project.order[0];
    let original = tests::key(&document, id);
    document
        .project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .exposure = 0.5;
    let graded = tests::key(&document, id);
    assert!(original.same_presentation(&graded));
    document.project.set_gamma(1.2);
    let interpreted = tests::key(&document, id);
    assert_ne!(original.recipe, interpreted.recipe);
    assert!(
        original.same_presentation(&interpreted),
        "interpretation updates can show an explicitly pending previous grade"
    );
    let mut other = graded.clone();
    other.target = OutputTarget::Srgb;
    assert!(!original.same_presentation(&other));
    other = graded.clone();
    let area = document.project.source(id).unwrap().development.active_area;
    other.field = Some(FieldKey(Arc::new(
        crate::flat_field::FlatField::from_nodes(area, [2, 2], vec![DVec3::splat(1.1); 4]).unwrap(),
    )));
    assert!(!original.same_presentation(&other));
    other = graded.clone();
    other.source.file = FileKey::new(std::path::Path::new("changed-source.ARW"));
    assert!(!original.same_presentation(&other));
    let uncropped = tests::key(&document, id);
    document.project.source_mut(id).unwrap().crop = Some(crate::crop::Crop {
        center: [128., 96.],
        size: [64, 64],
        angle: 0.,
    });
    let cropped = tests::key(&document, id);
    assert!(!cropped.same_presentation(&uncropped));
}

#[test]
fn cpu_look_charges_follow_node_allocations_not_equal_contents_or_wrapper_identity() {
    use crate::looks::{LookAsset, LookOrigin, Lut3D};
    let mut document: crate::editor::document_scope::DocumentScope = tests::fixture().into();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    let asset = |lut| Arc::new(LookAsset::new("Shared".into(), LookOrigin::Imported, lut).unwrap());
    let lut = Arc::new(Lut3D::new(2, [-0.5; 3], [1.1; 3], vec![[0.4, 0.3, 0.2]; 8]).unwrap());
    document.project.output.lut = Some(asset(lut.clone()));
    // Failed entries still retain their recipe's CPU look, but no GPU LUT.
    let mut entries: std::collections::VecDeque<_> = document.project.order[..2]
        .iter()
        .map(|&id| (tests::key(&document, id), Err("failed card".into())))
        .collect();
    let node_bytes = 8 * std::mem::size_of::<[f32; 3]>();
    assert_eq!(shared_look_bytes(&entries), node_bytes);
    document.project.output.lut = Some(asset(Arc::new((*lut).clone())));
    entries.push_back((
        tests::key(&document, document.project.order[2]),
        Err("failed card".into()),
    ));
    assert_eq!(
        shared_look_bytes(&entries),
        node_bytes,
        "cloned LUT wrappers share their Arc-backed nodes"
    );
    let distinct = Arc::new(Lut3D::new(2, [-0.5; 3], [1.1; 3], vec![[0.4, 0.3, 0.2]; 8]).unwrap());
    assert_eq!(distinct.id(), lut.id());
    document.project.output.lut = Some(asset(distinct));
    entries.push_back((
        tests::key(&document, document.project.order[3]),
        Err("failed card".into()),
    ));
    assert_eq!(
        shared_look_bytes(&entries),
        2 * node_bytes,
        "equal LUT values in distinct node allocations both consume storage"
    );
}

#[test]
fn document_thumbnails_finish_without_visible_cards_or_a_settled_main_view() {
    assert_document_cards_finish(false);
    assert_document_cards_finish(true);
}

fn assert_document_cards_finish(show_selected: bool) {
    let (device, queue) = gpu();
    let thumbnails = Thumbnails::with_disk(
        device.clone(),
        queue,
        Arc::new(crate::editor::display::DisplayFixture::for_device(&device)),
        None,
    );
    let directory = tempfile::tempdir().unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let paths: Vec<_> = (0..3)
        .map(|index| {
            let path = directory.path().join(format!("frame-{index}.dng"));
            std::fs::copy(&fixture, &path).unwrap();
            path
        })
        .collect();
    let mut document = Document::default();
    document.import(&paths).unwrap();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    let selected = document.project.order[0];
    let mut editor = Editor::state();
    editor.document = editor.document.replacement(document);
    editor.document.thumbnails = thumbnails.with_sources(editor.document.cache.clone());
    editor.set_cache_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 64 << 20,
    });
    editor.document.visit.current_source = Some(selected);
    // Keep the selected view unsettled throughout real thumbnail preparation.
    let frame = crate::frame_raster::FrameRaster::full(
        editor.thumbnail_composition(editor.document.state.project.source(selected).unwrap()),
    )
    .unwrap();
    let target = editor.document.target();
    editor.document.visit.working_view.observe(
        frame,
        crate::photographic::graph::request::PhotographRequest::Fixture(AuthoredPhotograph::from(
            ProcessingParameters::source(),
        )),
        target,
    );
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            if show_selected {
                editor.document.thumbnails.visible = vec![selected];
            }
            editor.advance_thumbnail_demand(cx);
        });
        app.run_until(Duration::from_secs(10), |editor| {
            editor.document.thumbnails.processed.attempted_sources.len() == 3
                && editor.document.thumbnails.processed.pending.is_none()
        });
        app.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .thumbnails
                    .ready_count(&editor.document, editor.document.target()),
                3,
                "offscreen positives do not wait for selected-view readiness"
            );
            assert!(
                editor
                    .document
                    .thumbnails
                    .processed
                    .cache
                    .contains(&tests::key(&editor.document, selected))
            );
        });
        app.read(|editor| {
            assert!(!editor.document.visit.working_view.idle());
            assert_eq!(
                editor.document.thumbnails.processed.attempted_sources.len(),
                3
            );
            assert!(
                !editor.document.thumbnails.processed.work.pending(),
                "completed eager cards must not loop back into preparation"
            );
        });
    });
}

#[test]
fn authored_36_positive_cards_with_a_shared_full_size_look_survive_reverse_browsing() {
    use crate::looks::{LookAsset, LookOrigin, Lut3D};
    let (device, queue) = gpu();
    let mut thumbnails = Thumbnails::with_disk(
        device.clone(),
        queue,
        Arc::new(crate::editor::display::DisplayFixture::for_device(&device)),
        None,
    );
    thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 256 << 20,
    });
    let directory = tempfile::tempdir().unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let paths: Vec<_> = (0..36)
        .map(|index| {
            let path = directory.path().join(format!("frame-{index}.dng"));
            std::fs::copy(&fixture, &path).unwrap();
            path
        })
        .collect();
    let mut document = Document::default();
    document.import(&paths).unwrap();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    // The production look grid is 129³. A constant authored LUT makes its resource
    // footprint realistic without bringing a built-in generator or photographs into this test.
    document.project.output.lut = Some(Arc::new(
        LookAsset::new(
            "Shared roll look".into(),
            LookOrigin::Imported,
            Arc::new(
                Lut3D::new(
                    129,
                    [-0.5; 3],
                    [1.1; 3],
                    vec![[0.4, 0.3, 0.2]; 129_usize.pow(3)],
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    ));
    let document: crate::editor::document_scope::DocumentScope = document.into();
    thumbnails = thumbnails.with_sources(document.cache.clone());
    for &id in &document.project.order {
        let product = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
        assert!(product.positive.is_some());
    }
    println!(
        "positive_roll ready={} entries={} charged_bytes={}",
        thumbnails.ready_count(&document, OutputTarget::DisplayP3),
        thumbnails.processed.cache.entries.len(),
        thumbnails.processed.cache.measured_bytes()
    );
    thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 192 << 20,
    });
    let mut next_target = tests::key(&document, document.project.order[0]);
    next_target.target = OutputTarget::Srgb;
    assert!(
        thumbnails.processed.can_retain(&next_target),
        "eager admission must not reserve another copy of an already-live LUT"
    );
    for &id in document
        .project
        .order
        .iter()
        .rev()
        .chain(document.project.order.iter())
    {
        let key = tests::key(&document, id);
        thumbnails.processed.protect_images(vec![key.clone()]);
        assert!(
            thumbnails
                .processed
                .cache
                .get(&key)
                .is_some_and(|entry| entry.is_ok()),
            "a developed roll card was evicted despite sufficient room for the actual shared resources"
        );
    }
    thumbnails.processed.protect_images(Vec::new());
    thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 0,
    });
    assert!(thumbnails.processed.cache.entries.is_empty());
    assert_eq!(
        thumbnails.processed.cache.measured_bytes(),
        0,
        "shared charges must retire with their cache owners"
    );
}

#[test]
fn repair_instructions_do_not_block_graded_navigation_cards() {
    let (device, queue) = gpu();
    let mut thumbnails = Thumbnails::with_disk(
        device.clone(),
        queue,
        Arc::new(crate::editor::display::DisplayFixture::for_device(&device)),
        None,
    );
    let mut document = document();
    let id = document.project.order[0];
    let area: crate::geometry::SourceRegion = document
        .project
        .source(id)
        .unwrap()
        .development
        .active_area
        .into();
    let clean_key = tests::key(&document, id);
    let support = SupportMask::new([area.x + 20, area.y + 20], [8, 8], vec![true; 64]).unwrap();
    let offset = nearest_offset(&support, area).unwrap();
    document
        .project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(support, RepairOrigin::Manual, offset)
        .unwrap();
    let key = tests::key(&document, id);
    assert_eq!(
        key.recipe.as_ref().unwrap().as_ref().unwrap().repairs.len(),
        1,
        "navigation captures complete intent even though proxy quality omits repair execution"
    );
    assert_ne!(key, clean_key, "a different intent requests a fresh card");
    assert!(!clean_key.current_recipe(&document));
    let card = complete(&mut thumbnails, &document, key.clone()).unwrap();
    assert!(card.positive.is_some());
    assert!(key.current_recipe(&document));
    assert_eq!(
        document.project.image_recipe(id).unwrap().repairs.len(),
        1,
        "navigation omission must not remove authoritative repair intent"
    );
}
