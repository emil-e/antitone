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
    document.project.dmin = Some([0.8; 3]);
    document.project.dmax = Some([0.02; 3]);
    document.project.input_floor = Some(crate::processing::CAMERA_INPUT_FLOOR);
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
fn authored_positive_thumbnails_require_originals_reuse_neutral_repairs_and_reject_stale_recipes() {
    let (device, queue) = gpu();
    let renderers = Arc::new(crate::renderer::RendererCache::default());
    let mut thumbnails = Thumbnails::with_disk(device, queue, renderers, None);
    let mut document = document();
    let id = document.project.order[0];
    let source = document.project.source(id).unwrap().clone();
    let scan = Scan::open(
        &document.source_path(id).unwrap(),
        Some(document.project.development.as_ref().unwrap().fixed_wb),
    )
    .unwrap();
    let proxy = Arc::new(Proxy::from_scan(&scan).unwrap());
    thumbnails.bind_sources(document.cache().clone());
    thumbnails.seed(
        &source,
        document.project.development.as_ref().unwrap(),
        FileKey::new(&document.source_path(id).unwrap()),
        proxy,
    );
    drop(scan);
    let plain_key = tests::key(&document, id);
    let started = std::time::Instant::now();
    let plain = complete(&mut thumbnails, &document, plain_key.clone()).unwrap();
    println!(
        "positive_thumbnail_cold_original_seconds={:.6}",
        started.elapsed().as_secs_f64()
    );
    assert!(
        plain.positive.is_some(),
        "a camera proxy alone is not a completed calibrated positive"
    );
    let repair = document
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
    let repaired_key = tests::key(&document, id);
    assert_ne!(plain_key, repaired_key);
    let repaired = complete(&mut thumbnails, &document, repaired_key.clone()).unwrap();
    let neutral = repaired.positive.as_ref().unwrap().0.clone();
    assert_eq!(neutral.spec.repairs.0.len(), 1);
    assert!(!Arc::ptr_eq(&plain.positive.as_ref().unwrap().0, &neutral));
    thumbnails.protect_images(vec![repaired_key.clone()]);
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
        "grade/target changes must reuse compatible neutral healing"
    );
    assert!(!Arc::ptr_eq(&repaired.image, &graded.image));
    assert!(
        graded_key.estimated_product_bytes().unwrap()
            >= graded.byte_len() + graded_key.retained_bytes()
    );
    thumbnails.protect_images(vec![graded_key.clone()]);
    assert!(
        !thumbnails.cache.contains(&repaired_key),
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
    document
        .project
        .source_mut(id)
        .unwrap()
        .repairs
        .set_enabled(repair, false)
        .unwrap();
    assert!(!repaired_key.current_recipe(&document));
    assert!(!graded_key.current_recipe(&document));
    let disabled_key = tests::key(&document, id);
    let disabled = complete(&mut thumbnails, &document, disabled_key).unwrap();
    assert!(
        disabled
            .positive
            .as_ref()
            .unwrap()
            .0
            .spec
            .repairs
            .0
            .is_empty()
    );
}

#[test]
fn repaired_thumbnails_capture_looks_reuse_neutral_and_grade_and_charge_retained_look_payloads() {
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
    assert_eq!(neutral.spec.repairs.0.len(), 1);
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
    let gamma = editor.document.project.roll.gamma;
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
        editor.document.project.roll.gamma = gamma;
        editor.roll.gamma = gamma;
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
fn previous_card_presentation_requires_the_same_source_repairs_framing_and_target() {
    let mut document = document();
    let id = document.project.order[0];
    let original = tests::key(&document, id);
    document.project.source_mut(id).unwrap().photo.exposure = 0.5;
    let graded = tests::key(&document, id);
    assert!(original.same_presentation(&graded));
    document.project.roll.gamma = 1.2;
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
    assert!(
        !original.same_presentation(&tests::key(&document, id)),
        "old unrepaired pixels are not compatible accepted repair feedback"
    );
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
fn authored_failed_thumbnail_repairs_publish_only_a_failure_under_the_current_recipe() {
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
    let error = complete(&mut thumbnails, &document, key.clone())
        .err()
        .expect("no donor must not produce an unrepaired positive");
    assert!(
        error.contains("repair 1") && error.contains("donor"),
        "{error}"
    );
    assert!(matches!(thumbnails.cache.get(&key), Some(Err(_))));
}
