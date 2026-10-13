use super::*;

fn cards(output: &egui::FullOutput) -> Vec<&egui::epaint::RectShape> {
    output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Rect(rect) = &shape.shape
                && rect.corner_radius == egui::CornerRadius::same(4)
                && rect.rect.width() >= 64.
                && rect.stroke.width > 0.
            {
                Some(rect)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn card_frames_and_captions_fit_compact_and_normal_strips_without_changing_preferences() {
    let (mut editor, _, _commands) = crate::editor::tests::fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor.document = super::tests::fixture();
    editor.current_source = Some(editor.document.project.order[0]);
    editor.busy = true;
    let before = editor.document.project.clone();
    for height in [52., 80., 140., 240.] {
        editor.preferences.filmstrip_height = height;
        for size in [egui::vec2(700., 600.), egui::vec2(1100., 800.)] {
            let context = egui::Context::default();
            for _ in 0..3 {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| editor.filmstrip(ui),
                );
                output.textures_delta.clear();
                let cards = cards(&output);
                assert!(!cards.is_empty());
                let panel = egui::containers::panel::PanelState::load(
                    &context,
                    egui::Id::new("thumbnail-filmstrip"),
                )
                .unwrap()
                .outer_rect;
                for card in &cards {
                    assert!(
                        card.rect.top() >= panel.top() && card.rect.bottom() <= panel.bottom(),
                        "{height}pt / {size:?} / panel={panel:?} / card={:?}",
                        card.rect
                    );
                }
                assert_eq!(
                    cards[0].stroke.color,
                    context
                        .style_of(egui::Theme::Dark)
                        .visuals
                        .selection
                        .stroke
                        .color
                );
                assert_eq!(cards[0].stroke.width, 1.5);
                assert_eq!(cards[0].fill, egui::Color32::from_gray(30));
                let captions: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape
                            && text.galley.job.sections[0].format.font_id.size == 11.
                        {
                            Some(text)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(captions.len(), cards.len());
                for (caption, card) in captions.iter().zip(cards.iter()) {
                    let bounds = caption.galley.rect.translate(caption.pos.to_vec2());
                    assert_eq!(caption.galley.rows.len(), 1);
                    assert!(card.rect.expand(0.5).contains_rect(bounds));
                    assert!((bounds.center().x - card.rect.center().x).abs() < 0.5);
                }
                assert_eq!(editor.preferences.filmstrip_height, height);
                assert_eq!(editor.document.project, before);
            }
        }
    }
}

#[test]
fn hover_is_neutral_and_distinct_from_selection_without_changing_cell_geometry() {
    let (mut editor, _, _commands) = crate::editor::tests::fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor.document = super::tests::fixture();
    editor.current_source = Some(editor.document.project.order[0]);
    editor.busy = true;
    let context = egui::Context::default();
    let mut frame = |pointer: Option<egui::Pos2>| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                events: pointer.into_iter().map(egui::Event::PointerMoved).collect(),
                ..Default::default()
            },
            |ui| editor.filmstrip(ui),
        );
        output.textures_delta.clear();
        output
    };
    let idle = frame(None);
    let point = cards(&idle)[1].rect.center();
    let hover = frame(Some(point));
    let idle = cards(&idle);
    let hover = cards(&hover);
    assert_eq!(idle[1].rect, hover[1].rect);
    assert_eq!(hover[1].fill, egui::Color32::from_gray(28));
    assert_eq!(hover[1].stroke.color, egui::Color32::from_gray(65));
    assert_ne!(hover[1].stroke, hover[0].stroke);
    assert_eq!(hover[0].fill, egui::Color32::from_gray(30));
}

#[test]
#[ignore = "writes private GPU-painted review fixtures to .cache/probes/camera-roll"]
fn capture_camera_roll_review_fixtures() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let shared = crate::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let pixels: Vec<[u16; 4]> = (0..65)
            .flat_map(|y| {
                (0..97).map(move |x| {
                    let sky = y < 28 + x / 8;
                    if sky {
                        [11000 + x * 80, 18000 + y * 180, 30000 + x * 120, 0]
                    } else {
                        [19000 + x * 100, 17000 + y * 150, 7000 + x * 60, 0]
                    }
                })
            })
            .collect();
        let image = Arc::new(
            GpuImage::new_shared(
                &shared,
                &device,
                97,
                65,
                &pixels,
                glam::DMat3::IDENTITY,
                OutputTarget::DisplayP3,
            )
            .await
            .unwrap(),
        );
        let (mut editor, _, _commands) = crate::editor::tests::fixture();
        editor.document = super::tests::fixture();
        editor.document.project.order.truncate(6);
        let order = editor.document.project.order.clone();
        for (id, name) in order.iter().zip([
            "DSC04397.ARW",
            "DSC04416.ARW",
            "DSC04378.ARW",
            "Summer_Scan_Reference_Roll01_Frame_04397.ARW",
            "Developing_04398.ARW",
            "Unavailable_04399.ARW",
        ]) {
            editor.document.project.source_mut(*id).unwrap().path = name.into();
        }
        editor
            .document
            .project
            .source_mut(order[1])
            .unwrap()
            .orientation
            .turn_clockwise();
        editor.document.project.source_mut(order[2]).unwrap().crop = Some(Crop {
            center: [450., 300.],
            size: [800, 300],
            angle: 0.,
        });
        for id in &order[..4] {
            editor
                .thumbnails
                .cache
                .insert(super::tests::key(&editor.document, *id), Ok(image.clone()));
        }
        editor.thumbnails.cache.insert(
            super::tests::key(&editor.document, order[5]),
            Err("Source unavailable: review fixture".into()),
        );
        editor.current_source = Some(order[0]);
        editor.dmin = None;
        editor.dmax = None;
        let record = editor.document.project.source(order[0]).unwrap().clone();
        editor.ready = Some(Ready {
            thumbnail: None,
            path: record.path.clone(),
            area: record.development.active_area.into(),
            analysis_support: None,
            floor: editor.floor,
            images: [image.clone(), image],
            record,
            development: editor.document.project.development.clone().unwrap(),
        });
        let directory = PathBuf::from(".cache/probes/camera-roll");
        std::fs::create_dir_all(&directory).unwrap();
        for height in [140., 80., 52.] {
            editor.preferences.filmstrip_height = height;
            let context = egui::Context::default();
            context.set_pixels_per_point(2.);
            let mut renderer = egui_wgpu::Renderer::new(
                &device,
                wgpu::TextureFormat::Rgba16Float,
                Default::default(),
            );
            let mut capture = Vec::new();
            for step in 0..3 {
                let output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1024., 800.),
                        )),
                        events: vec![egui::Event::PointerMoved(egui::pos2(
                            if height == 140. { 240. } else { 100. },
                            800. - height * 0.5,
                        ))],
                        time: Some(f64::from(step) / 60.),
                        ..Default::default()
                    },
                    |ui| editor.filmstrip(ui),
                );
                capture = super::clipping_tests::paint(
                    &device,
                    &queue,
                    &context,
                    &mut renderer,
                    output,
                    true,
                    [1024, 800],
                );
            }
            let width = 2048;
            let first_row = (800. - height) as usize * 2;
            let bytes: Vec<u8> = capture[first_row * width..]
                .iter()
                .flat_map(|pixel| {
                    pixel[..3].iter().map(|bits| {
                        (half::f16::from_bits(*bits).to_f32().clamp(0., 1.) * 255.).round() as u8
                    })
                })
                .collect();
            let path = directory.join(format!("fixture-{height:.0}.tif"));
            let file = std::fs::File::create(&path).unwrap();
            tiff::encoder::TiffEncoder::new(file)
                .unwrap()
                .write_image::<tiff::encoder::colortype::RGB8>(
                    width as u32,
                    height as u32 * 2,
                    &bytes,
                )
                .unwrap();
            println!("GPU-painted synthetic review fixture: {}", path.display());
        }
    });
}
