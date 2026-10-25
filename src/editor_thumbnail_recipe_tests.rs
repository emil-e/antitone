use super::*;
use crate::dust_repair::{DonorChoice, RepairOrigin, SupportMask};

#[cfg(feature = "ocio-reference")]
#[path = "editor_thumbnail_pixel_tests.rs"]
mod pixel_tests;

fn gpu() -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        adapter.request_device(&Default::default()).await.unwrap()
    })
}

fn document() -> Document {
    let mut document = Document::default();
    document
        .import(&[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng")])
        .unwrap();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    document
        .project
        .set_input_floor(Some(crate::processing::CAMERA_INPUT_FLOOR));
    document
}

fn complete(
    thumbnails: &mut Thumbnails,
    document: &Document,
    key: ImageKey,
) -> Result<Arc<Product>, String> {
    let id = key.source.id;
    thumbnails.protect_images(vec![key.clone()]);
    let job = tests::service_job(
        thumbnails,
        key.clone(),
        document.source_path(id).unwrap(),
        document.project.source(id).unwrap().clone(),
        None,
        true,
    );
    tests::complete_service_request(thumbnails, document, job);
    thumbnails.cache.get(&key).expect("current result").clone()
}

#[test]
fn authored_positive_cards_keep_grade_reuse_and_freshness() {
    let (device, queue) = gpu();
    let renderers = Arc::new(crate::renderer::RendererCache::default());
    let mut thumbnails = Thumbnails::with_disk(device, queue, renderers, None);
    let mut document = document();
    let id = document.project.order[0];
    let source = document.project.source(id).unwrap().clone();
    let scan = Scan::open(
        &document.source_path(id).unwrap(),
        Some(document.project.development().as_ref().unwrap().fixed_wb),
    )
    .unwrap();
    let proxy = Arc::new(Proxy::from_scan(&scan).unwrap());
    thumbnails.bind_sources(document.cache().clone());
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
    let neutral = plain.positive.as_ref().unwrap().0.clone();
    thumbnails.protect_images(vec![plain_key.clone()]);
    thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 0,
    });
    document.project.source_mut(id).unwrap().photo.exposure = 0.5;
    let mut locals = crate::project::LocalAdjustments::default();
    locals
        .add(
            "Linear".into(),
            crate::local_adjustments::LocalAdjustment {
                mask: crate::local_adjustments::GradientMask::Linear {
                    start: crate::geometry::SourcePoint([0., 0.]),
                    end: crate::geometry::SourcePoint([256., 192.]),
                },
                photo: PhotoSettings {
                    exposure: 0.75,
                    ..Default::default()
                },
                enabled: true,
                invert: false,
                strength: 1.,
            },
        )
        .unwrap();
    document.project.source_mut(id).unwrap().local_adjustments = Some(locals);
    let mut graded_key = tests::key(&document, id);
    graded_key.target = OutputTarget::Srgb;
    let graded = complete(&mut thumbnails, &document, graded_key.clone()).unwrap();
    assert!(
        Arc::ptr_eq(&neutral, &graded.positive.as_ref().unwrap().0),
        "grade/target changes must reuse the neutral proxy"
    );
    assert!(!Arc::ptr_eq(&plain.image, &graded.image));
    assert!(
        graded_key.estimated_product_bytes().unwrap()
            >= graded.byte_len() + graded_key.retained_bytes()
    );
    thumbnails.protect_images(vec![graded_key.clone()]);
    assert!(
        !thumbnails.cache.contains(&plain_key),
        "replacement completion must retire the carried old card at zero optional retention"
    );
    document.project.source_mut(id).unwrap().photo.exposure = 1.;
    let failed_key = tests::key(&document, id);
    thumbnails.protect_images(vec![failed_key.clone()]);
    thumbnails
        .cache
        .insert(failed_key.clone(), Err("grade range rejection".into()));
    thumbnails.protect_images(vec![failed_key]);
    assert!(
        thumbnails.cache.contains(&graded_key),
        "a failed downstream grade must not discard compatible visible neutral content"
    );
    document.project.source_mut(id).unwrap().photo.exposure = 0.25;
    let readmitted = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    assert!(Arc::ptr_eq(
        &neutral,
        &readmitted.positive.as_ref().unwrap().0
    ));

    assert!(
        !graded_key.current_recipe(&document),
        "a changed grade must still reject stale results"
    );
}

#[test]
fn cached_proxy_finishes_repair_bearing_navigation_cards_without_a_developable_raw() {
    let (device, queue) = gpu();
    let mut thumbnails = Thumbnails::with_disk(
        device,
        queue,
        Arc::new(crate::renderer::RendererCache::default()),
        None,
    );
    // This document has locator-only files, not decodable RAWs: the published
    // camera proxy is the only available image source.
    let mut document = tests::fixture();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    document
        .project
        .set_input_floor(Some(crate::processing::CAMERA_INPUT_FLOOR));
    let id = document.project.order[0];
    thumbnails.bind_sources(document.cache().clone());
    thumbnails.seed(
        document.project.source(id).unwrap(),
        FileKey::new(&document.source_path(id).unwrap()),
        Arc::new(Proxy {
            width: 8,
            height: 6,
            pixels: vec![[5000., 7000., 9000., 0.]; 48],
            matrix: glam::DMat3::IDENTITY,
        }),
    );
    let plain = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    assert!(plain.positive.is_some());
    document.project.source_mut(id).unwrap().photo.exposure = 0.5;
    let graded = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    assert!(
        Arc::ptr_eq(
            &plain.positive.as_ref().unwrap().0,
            &graded.positive.as_ref().unwrap().0
        ),
        "grading must reuse the small neutral source, not request originals"
    );
    document
        .project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([120, 60], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Offset([-64, 0]),
        )
        .unwrap();
    thumbnails.cache.retain(|_, _| false);
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
        device,
        queue,
        Arc::new(crate::renderer::RendererCache::default()),
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
            DonorChoice::Offset([-64, 0]),
        )
        .unwrap();
    let plain = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    let neutral = plain.positive.as_ref().unwrap().0.clone();
    thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 0,
    });
    let lut = Arc::new(Lut3D::new(2, [-0.5; 3], [1.1; 3], vec![[0.4, 0.3, 0.2]; 8]).unwrap());
    document.project.output.look = Some(Arc::new(
        LookAsset::new("Card look".into(), LookOrigin::Imported, lut.clone()).unwrap(),
    ));
    let key = tests::key(&document, id);
    let looked = complete(&mut thumbnails, &document, key.clone()).unwrap();
    let view = &looked.positive.as_ref().unwrap().1;
    assert_eq!(view.processing.look_id(), Some(lut.id()));
    assert!(matches!(looked.look_result(), Some(Ok(()))));
    assert!(Arc::ptr_eq(&neutral, &looked.positive.as_ref().unwrap().0));
    #[cfg(feature = "ui-tests")]
    assert!(view.same_grade_as(&plain.positive.as_ref().unwrap().1));
    assert!(looked.byte_len() > plain.byte_len());
    assert!(key.estimated_product_bytes().unwrap() >= key.retained_bytes() + looked.byte_len());
    let mut draft = key.clone();
    draft.projection = Some(
        Composition::with_crop(
            document
                .project
                .source(id)
                .unwrap()
                .development
                .active_area
                .into(),
            document.project.orientation,
            document.project.source(id).unwrap().orientation,
            Some(crate::crop::Crop {
                center: [64., 64.],
                size: [64, 64],
                angle: 0.2,
            }),
        )
        .unwrap(),
    );
    assert_ne!(draft, key);
    let framed = complete(&mut thumbnails, &document, draft).unwrap();
    assert!(Arc::ptr_eq(&neutral, &framed.positive.as_ref().unwrap().0));
    #[cfg(feature = "ui-tests")]
    assert!(view.same_grade_as(&framed.positive.as_ref().unwrap().1));
    assert!(matches!(framed.look_result(), Some(Ok(()))));
    document.project.output.look = None;
    assert!(!key.current_recipe(&document));
    let restored = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    assert!(Arc::ptr_eq(
        &neutral,
        &restored.positive.as_ref().unwrap().0
    ));
    assert_eq!(
        restored.positive.as_ref().unwrap().1.processing.look_id(),
        None
    );
}

#[test]
fn grading_keeps_the_previous_card_visible_and_replacement_does_not_accumulate_versions() {
    let (device, queue) = gpu();
    let mut thumbnails = Thumbnails::with_disk(
        device,
        queue,
        Arc::new(crate::renderer::RendererCache::default()),
        None,
    );
    let document = document();
    let id = document.project.order[0];
    let initial_key = tests::key(&document, id);
    complete(&mut thumbnails, &document, initial_key.clone()).unwrap();
    let mut editor = Editor::state();
    editor.document = document.into();
    editor.thumbnails = thumbnails;
    editor.load_input(editor.document.project.input);
    editor.current_source = Some(id);
    // Hold asynchronous replacement so the composed filmstrip must use existing pixels.
    editor.busy = true;
    let context = egui::Context::default();
    context.enable_accesskit();
    let frame = |editor: &mut Editor| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                ..Default::default()
            },
            |ui| editor.filmstrip(ui, &crate::application::UiContext::render_only()),
        );
        output.textures_delta.clear();
        output
    };
    let gamma = editor.document.project.input.gamma();
    for (index, (exposure, gamma)) in [(0.25, gamma), (0.5, gamma), (0.75, gamma), (1., 1.2)]
        .into_iter()
        .enumerate()
    {
        if index == 3 {
            editor.thumbnails.set_limits(cache_limits::CacheLimits {
                cpu_bytes: 0,
                gpu_bytes: 0,
            });
        }
        editor.document.project.set_gamma(gamma);
        editor.set_gamma(gamma);
        editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .photo
            .exposure = exposure;
        editor.photo.exposure = exposure;
        let output = frame(&mut editor);
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Callback(_))),
            "grading must not replace the existing image with a blank/JPEG"
        );
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.job.text == "…")));
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
        let key = tests::key(&editor.document, id);
        complete(&mut editor.thumbnails, &editor.document, key.clone()).unwrap();
        assert_eq!(
            editor.thumbnails.cache.entries.len(),
            1,
            "superseded grades must not compete with other roll photos for retention"
        );
        assert!(editor.thumbnails.cache.contains(&key));
        let output = frame(&mut editor);
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
    editor
        .document
        .project
        .source_mut(id)
        .unwrap()
        .photo
        .exposure = 1.25;
    let key = tests::key(&editor.document, id);
    editor.thumbnails.protect_images(vec![key.clone()]);
    editor
        .thumbnails
        .cache
        .insert(key, Err("render rejected".into()));
    let output = frame(&mut editor);
    assert!(
        !output
            .shapes
            .iter()
            .any(|shape| matches!(shape.shape, egui::Shape::Callback(_))),
        "an authoritative error must not be hidden by old pixels"
    );
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.job.text == "!")));
}

#[test]
fn replacing_a_grade_preserves_other_photos_in_the_roll_cache() {
    let (device, queue) = gpu();
    let mut thumbnails = Thumbnails::with_disk(
        device,
        queue,
        Arc::new(crate::renderer::RendererCache::default()),
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
    document.project.source_mut(first).unwrap().photo.exposure = 0.5;
    let replacement = tests::key(&document, first);
    complete(&mut thumbnails, &document, replacement.clone()).unwrap();
    assert!(!thumbnails.cache.contains(&first_key));
    assert!(thumbnails.cache.contains(&replacement));
    assert!(
        Arc::ptr_eq(
            thumbnails.cache.get(&other_key).unwrap().as_ref().unwrap(),
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
    document.project.source_mut(id).unwrap().photo.exposure = 0.5;
    let graded = tests::key(&document, id);
    assert!(original.same_presentation(&graded));
    document.project.set_gamma(1.2);
    let interpreted = tests::key(&document, id);
    assert!(!original.same_neutral(&interpreted));
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
    let mut projected = uncropped.clone();
    projected.projection = Some(
        cropped
            .recipe
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .composition,
    );
    assert!(
        !projected.same_presentation(&uncropped),
        "transient tool framing must not reuse a card validated for another footprint"
    );
}

#[test]
fn cpu_look_charges_follow_node_allocations_not_equal_contents_or_wrapper_identity() {
    use crate::looks::{LookAsset, LookOrigin, Lut3D};
    let mut document = tests::fixture();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    document
        .project
        .set_input_floor(Some(crate::processing::CAMERA_INPUT_FLOOR));
    let asset = |lut| Arc::new(LookAsset::new("Shared".into(), LookOrigin::Imported, lut).unwrap());
    let lut = Arc::new(Lut3D::new(2, [-0.5; 3], [1.1; 3], vec![[0.4, 0.3, 0.2]; 8]).unwrap());
    document.project.output.look = Some(asset(lut.clone()));
    // Failed entries still retain their recipe's CPU look, but no GPU LUT.
    let mut entries: std::collections::VecDeque<_> = document.project.order[..2]
        .iter()
        .map(|&id| (tests::key(&document, id), Err("failed card".into())))
        .collect();
    let node_bytes = 8 * std::mem::size_of::<[f32; 3]>();
    assert_eq!(shared_look_bytes(&entries), node_bytes);
    document.project.output.look = Some(asset(Arc::new((*lut).clone())));
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
    document.project.output.look = Some(asset(distinct));
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
fn authored_offscreen_cpu_completion_advances_to_positive_cards_when_the_view_becomes_idle() {
    let (device, queue) = gpu();
    let thumbnails = Thumbnails::with_disk(
        device,
        queue,
        Arc::new(crate::renderer::RendererCache::default()),
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
    document
        .project
        .set_input_floor(Some(crate::processing::CAMERA_INPUT_FLOOR));
    let selected = document.project.order[0];
    let mut editor = Editor::state();
    editor.document = document.into();
    editor.thumbnails = thumbnails;
    editor
        .thumbnails
        .bind_sources(editor.document.cache().clone());
    editor.thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 64 << 20,
    });
    editor.current_source = Some(selected);
    // An unsettled selected-view intent blocks optional offscreen GPU work, not
    // the actual thumbnail/native services or the selected card's visible work.
    let frame = crate::frame_raster::FrameRaster::full(
        editor.thumbnail_composition(editor.document.project.source(selected).unwrap()),
    )
    .unwrap();
    editor.working.observe(
        editor.id,
        frame,
        ProcessingInput::from(ProcessingParameters::source()).signature(),
        editor.target,
    );
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.thumbnails.demand = Demand::Visible(vec![selected]);
            editor.advance_thumbnail_demand(cx);
        });
        app.run_until(Duration::from_secs(10), |editor| {
            editor.thumbnails.attempted_sources.len() == 3 && editor.thumbnails.pending.is_none()
        });
        app.read(|editor| {
            assert_eq!(
                editor
                    .thumbnails
                    .ready_count(&editor.document, editor.target),
                1,
                "the selected positive completes while offscreen cards are CPU-only"
            );
            assert!(
                editor
                    .thumbnails
                    .cache
                    .contains(&tests::key(&editor.document, selected))
            );
        });
        app.enter(|editor, cx| {
            editor.working.clear();
            editor.original.last_edit = std::time::Instant::now() - Duration::from_secs(1);
            editor.advance_thumbnail_demand(cx);
        });
        app.run_until(Duration::from_secs(10), |editor| {
            editor
                .thumbnails
                .ready_count(&editor.document, editor.target)
                == 3
                && editor.thumbnails.pending.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.thumbnails.attempted_sources.len(), 3);
            assert!(
                !editor.document.thumbnail_work.pending(),
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
        device,
        queue,
        Arc::new(crate::renderer::RendererCache::default()),
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
    document
        .project
        .set_input_floor(Some(crate::processing::CAMERA_INPUT_FLOOR));
    // The production look grid is 129³. A constant authored LUT makes its resource
    // footprint realistic without bringing a built-in generator or photographs into this test.
    document.project.output.look = Some(Arc::new(
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
    thumbnails.bind_sources(document.cache().clone());
    for &id in &document.project.order {
        let product = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
        assert!(product.positive.is_some());
    }
    println!(
        "positive_roll ready={} entries={} charged_bytes={}",
        thumbnails.ready_count(&document, OutputTarget::DisplayP3),
        thumbnails.cache.entries.len(),
        thumbnails.cache.measured_bytes()
    );
    thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 192 << 20,
    });
    let mut next_target = tests::key(&document, document.project.order[0]);
    next_target.target = OutputTarget::Srgb;
    assert!(
        thumbnails.can_retain(&next_target),
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
        thumbnails.protect_images(vec![key.clone()]);
        assert!(
            thumbnails
                .cache
                .get(&key)
                .is_some_and(|entry| entry.is_ok()),
            "a developed roll card was evicted despite sufficient room for the actual shared resources"
        );
    }
    thumbnails.protect_images(Vec::new());
    thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 0,
    });
    assert!(thumbnails.cache.entries.is_empty());
    assert_eq!(
        thumbnails.cache.measured_bytes(),
        0,
        "shared charges must retire with their cache owners"
    );
}

#[test]
fn no_donor_repair_instructions_do_not_block_graded_navigation_cards() {
    let (device, queue) = gpu();
    let mut thumbnails = Thumbnails::with_disk(
        device,
        queue,
        Arc::new(crate::renderer::RendererCache::default()),
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
    // The inset ring admits an exterior but its padded donor rectangle spans the source.
    document
        .project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new(
                [area.x, area.y],
                [area.width, area.height],
                (0..area.width * area.height)
                    .map(|index| {
                        let x = index % area.width;
                        let y = index / area.width;
                        x >= 8
                            && y >= 8
                            && x + 8 < area.width
                            && y + 8 < area.height
                            && (x == 8 || y == 8 || x + 9 == area.width || y + 9 == area.height)
                    })
                    .collect(),
            )
            .unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Automatic,
        )
        .unwrap();
    let key = tests::key(&document, id);
    let card = complete(&mut thumbnails, &document, key.clone()).unwrap();
    assert!(card.positive.is_some());
    assert!(key.current_recipe(&document));
    assert_eq!(
        document.project.image_recipe(id).unwrap().repairs().len(),
        1,
        "navigation omission must not remove authoritative repair intent"
    );
}
