use super::super::tests::{fixture, key};
use super::*;

fn jpeg(width: u32, height: u32, flip: i32) -> crate::raw::EmbeddedThumbnail {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
        .encode(
            &vec![127; width as usize * height as usize * 3],
            width,
            height,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    crate::raw::EmbeddedThumbnail {
        jpeg: bytes,
        flip,
        color_space: 1,
    }
}

#[test]
fn authored_dng_extraction_reaches_the_oriented_display_referred_placeholder_decoder() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/preview.dng");
    let mut raw = crate::raw::RawFile::open(&path).unwrap();
    let thumbnail = raw.embedded_thumbnail().unwrap().unwrap();
    let image = decode(thumbnail).unwrap().unwrap();
    assert_eq!(image.size, [8, 16]);
    for y in 0..16 {
        let code = if y < 8 { 64 } else { 192 };
        for x in 0..8 {
            assert_eq!(
                image.pixels[y * 8 + x],
                egui::Color32::from_rgb(code, code, code)
            );
        }
    }
    assert!(
        raw.camera_image().is_err(),
        "a camera JPEG is not developed/inverted RAW"
    );
}

#[test]
fn authored_dng_preview_worker_installs_and_releases_the_visible_placeholder() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/preview.dng");
    let mut document = Document::default();
    document.import(&[path]).unwrap();
    let context = egui::Context::default();
    let mut previews = EmbeddedPreviews::new(Arc::new(std::sync::OnceLock::new()));
    let visible = vec![wanted(&document, 0)];
    previews.request_visible(&visible, true);
    let started = std::time::Instant::now();
    while previews.pending() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "JPEG worker did not complete"
        );
        previews.receive(&document, &context);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(previews.get(&visible[0].1.source).unwrap().size(), [8, 16]);
    // The proper-thumbnail coordinator removes replaced images from this wanted list.
    previews.request_visible(&[], true);
    assert!(previews.entries.is_empty());
    assert!(!previews.pending());
}

#[test]
fn jpeg_decode_is_bounded_oriented_and_display_referred_without_upscaling() {
    let normal = decode(jpeg(8, 4, 0)).unwrap().unwrap();
    assert_eq!(normal.size, [8, 4]);
    for pixel in &normal.pixels {
        assert_eq!(*pixel, egui::Color32::from_rgb(127, 127, 127));
    }
    let rotated = decode(jpeg(8, 4, 6)).unwrap().unwrap();
    assert_eq!(rotated.size, [4, 8]);
    let reduced = decode(jpeg(400, 200, 5)).unwrap().unwrap();
    assert_eq!(reduced.size, [128, 256]);
    assert!(decode(jpeg(2049, 1, 0)).is_err());
    assert!(
        decode(crate::raw::EmbeddedThumbnail {
            jpeg: vec![1, 2, 3],
            flip: 0,
            color_space: 1
        })
        .is_err()
    );
    let mut unknown = jpeg(8, 4, 0);
    unknown.color_space = 65535;
    assert!(decode(unknown).unwrap().is_none());
    assert!(decode(jpeg(8, 4, 9)).unwrap().is_none());
}

#[test]
fn embedded_icc_and_exif_are_honoured_without_camera_inversion() {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95);
    let icc = lcms2::Profile::new_srgb().icc().unwrap();
    encoder.set_icc_profile(icc).unwrap();
    // Little-endian TIFF IFD with explicit EXIF orientation 1: overrides the native fallback flip.
    encoder
        .set_exif_metadata(vec![
            b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
        ])
        .unwrap();
    encoder
        .encode(&[127; 8 * 4 * 3], 8, 4, image::ExtendedColorType::Rgb8)
        .unwrap();
    let image = decode(crate::raw::EmbeddedThumbnail {
        jpeg: bytes,
        flip: 6,
        color_space: 65535,
    })
    .unwrap()
    .unwrap();
    assert_eq!(image.size, [8, 4]);
    assert_eq!(image.pixels[0], egui::Color32::from_rgb(127, 127, 127));
}

fn state() -> (
    EmbeddedPreviews,
    mpsc::Receiver<Request>,
    mpsc::Sender<ResultMessage>,
) {
    let (commands, requests) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    (
        EmbeddedPreviews {
            commands: Some(commands),
            messages: Some(messages),
            ..Default::default()
        },
        requests,
        send,
    )
}

fn wanted(document: &Document, index: usize) -> (usize, ImageKey, PathBuf, Source) {
    let id = document.project.order[index];
    (
        index,
        key(document, id),
        document.source_path(id).unwrap(),
        document.project.source(id).unwrap().clone(),
    )
}

fn complete(request: Request, send: &mpsc::Sender<ResultMessage>, available: bool) {
    send.send(ResultMessage {
        ticket: request.ticket,
        key: request.key,
        image: Ok(available.then(|| egui::ColorImage::filled([2, 1], egui::Color32::RED))),
    })
    .unwrap();
}

#[test]
fn visible_jpeg_pass_is_independent_bounded_and_drops_completed_or_offscreen_placeholders() {
    let document = fixture();
    let context = egui::Context::default();
    let (mut previews, requests, send) = state();
    let wanted = vec![wanted(&document, 0), wanted(&document, 1)];
    previews.request_visible(&wanted, true);
    let first = requests.try_recv().unwrap();
    previews.request_visible(&wanted, true);
    assert!(
        requests.try_recv().is_err(),
        "only one JPEG job, independently of the RAW worker"
    );
    complete(first, &send, true);
    previews.receive(&document, &context);
    assert!(previews.get(&wanted[0].1.source).is_some());
    previews.request_visible(&wanted, true);
    complete(requests.try_recv().unwrap(), &send, true);
    previews.receive(&document, &context);
    assert!(previews.get(&wanted[1].1.source).is_some());
    previews.request_visible(&wanted[1..], true);
    assert!(previews.get(&wanted[0].1.source).is_none());
    previews.request_visible(&[], true);
    assert!(
        previews.entries.is_empty(),
        "no recent/offscreen JPEG retention after proper replacement"
    );
}

#[test]
fn unavailable_cancelled_and_disconnected_previews_do_not_retry_or_block_raw_work() {
    let document = fixture();
    let context = egui::Context::default();
    let wanted = vec![wanted(&document, 0)];
    let (mut previews, requests, send) = state();
    previews.request_visible(&wanted, false);
    assert!(requests.try_recv().is_err());
    previews.request_visible(&wanted, true);
    complete(requests.try_recv().unwrap(), &send, false);
    previews.receive(&document, &context);
    previews.request_visible(&wanted, true);
    assert!(
        requests.try_recv().is_err(),
        "missing preview does not loop on each frame"
    );
    previews.entries.clear();
    previews.request_visible(&wanted, true);
    let request = requests.try_recv().unwrap();
    previews.request_visible(&wanted, false);
    assert!(previews.pending(), "cancellation cannot create a backlog");
    complete(request, &send, true);
    previews.receive(&document, &context);
    assert!(previews.entries.is_empty());
    previews.request_visible(&wanted, true);
    let request = requests.try_recv().unwrap();
    drop(send);
    previews.receive(&document, &context);
    assert!(!previews.pending());
    assert!(previews.commands.is_none());
    assert_eq!(previews.entries[0].0, request.key);
}

#[test]
fn critical_pressure_cancels_jpeg_installation_but_preserves_visible_textures() {
    let document = fixture();
    let context = egui::Context::default();
    let wanted = vec![wanted(&document, 0), wanted(&document, 1)];
    let (previews, requests, send) = state();
    let mut thumbnails = Thumbnails {
        embedded: previews,
        ..Default::default()
    };
    thumbnails.embedded.entries.push((
        wanted[0].1.source.clone(),
        Some(context.load_texture(
            "visible-camera",
            egui::ColorImage::filled([2, 1], egui::Color32::RED),
            egui::TextureOptions::LINEAR,
        )),
    ));
    thumbnails.embedded.request_visible(&wanted, true);
    let request = requests.try_recv().unwrap();
    thumbnails.apply_budgets(memory::CacheBudgets {
        images: 0,
        sources: 0,
        pressure: memory::Pressure::Critical,
    });
    complete(request, &send, true);
    thumbnails.embedded.receive(&document, &context);
    thumbnails.embedded.request_visible(&wanted, false);
    assert!(thumbnails.embedded.get(&wanted[0].1.source).is_some());
    assert!(thumbnails.embedded.get(&wanted[1].1.source).is_none());
    assert!(requests.try_recv().is_err());
}

#[test]
fn stale_document_source_stats_and_development_reject_previews() {
    let context = egui::Context::default();
    for stale in ["document", "source", "revision", "development", "cancel"] {
        let mut document = fixture();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("photo.ARW");
        std::fs::write(&path, b"initial").unwrap();
        let id = document.project.order[0];
        document.project.source_mut(id).unwrap().path = path.clone();
        let wanted = vec![wanted(&document, 0)];
        let (mut previews, requests, send) = state();
        previews.request_visible(&wanted, true);
        let request = requests.try_recv().unwrap();
        match stale {
            "document" => document = Document::default(),
            "source" => document.project.source_mut(id).unwrap().reference_only = true,
            "revision" => std::fs::write(&path, b"changed-bytes").unwrap(),
            "development" => document.project.development.as_mut().unwrap().fixed_wb[0] += 0.5,
            "cancel" => previews.cancel(),
            _ => unreachable!(),
        }
        complete(request, &send, true);
        previews.receive(&document, &context);
        assert!(!previews.pending());
        assert!(previews.entries.is_empty(), "{stale} must not install");
    }
}

#[test]
fn camera_images_and_badges_fit_polished_wells_and_clip_without_squeezing_or_hiding_filenames() {
    let mut edges = [false; 2];
    for height in [52., 80., 140., 240.] {
        for scale in [1., 2.] {
            for width in [700., 1100.] {
                let (mut editor, _, _commands) = crate::editor::tests::fixture();
                editor.document = fixture();
                editor.busy = true;
                editor.preferences.filmstrip_height = height;
                let before = editor.document.project.clone();
                let context = egui::Context::default();
                context.set_pixels_per_point(scale);
                let textures: Vec<_> = editor
                    .document
                    .project
                    .order
                    .iter()
                    .enumerate()
                    .map(|(index, id)| {
                        let dimensions = [[160, 120], [120, 160], [160, 32]][index % 3];
                        (
                            key(&editor.document, *id).source,
                            context.load_texture(
                                "camera-layout",
                                egui::ColorImage::filled(dimensions, egui::Color32::RED),
                                egui::TextureOptions::LINEAR,
                            ),
                        )
                    })
                    .collect();
                let mut frame = |time: f64, scroll: f32| {
                    editor.thumbnails.embedded.entries = textures
                        .iter()
                        .map(|(key, texture)| (key.clone(), Some(texture.clone())))
                        .collect();
                    let mut output = context.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 800.),
                            )),
                            time: Some(time),
                            events: vec![
                                egui::Event::PointerMoved(egui::pos2(256., 800. - height * 0.5)),
                                egui::Event::MouseWheel {
                                    unit: egui::MouseWheelUnit::Point,
                                    phase: egui::TouchPhase::Move,
                                    delta: egui::vec2(scroll, 0.),
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                            ..Default::default()
                        },
                        |ui| editor.filmstrip(ui),
                    );
                    output.textures_delta.clear();
                    output
                };
                let mut output = frame(0., 0.);
                for step in 1..3 {
                    output = frame(f64::from(step) / 60., 0.);
                }
                let shift = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh)
                            if textures
                                .iter()
                                .any(|(_, texture)| texture.id() == mesh.texture_id) =>
                        {
                            Some(mesh.calc_bounds().center().x - shape.clip_rect.left())
                        }
                        _ => None,
                    })
                    .unwrap();
                for step in 3..40 {
                    output = frame(f64::from(step) / 60., if step == 3 { -shift } else { 0. });
                }
                let wells: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        let egui::Shape::Rect(rect) = &shape.shape else {
                            return None;
                        };
                        if rect.stroke.width != 0. {
                            return None;
                        }
                        let inside_card = output.shapes.iter().any(|shape| match &shape.shape {
                            egui::Shape::Rect(card) => {
                                card.stroke.width > 0. && card.rect.contains_rect(rect.rect)
                            }
                            _ => false,
                        });
                        let contains_camera_image = output.shapes.iter().any(|shape| {
                            let egui::Shape::Mesh(mesh) = &shape.shape else {
                                return false;
                            };
                            textures
                                .iter()
                                .any(|(_, texture)| texture.id() == mesh.texture_id)
                                && rect.rect.expand(0.5).contains_rect(mesh.calc_bounds())
                        });
                        (inside_card && contains_camera_image).then_some(rect.rect)
                    })
                    .collect();
                let mut images = 0;
                let mut badges = 0;
                let mut captions = 0;
                for shape in &output.shapes {
                    match &shape.shape {
                        egui::Shape::Mesh(mesh) => {
                            let Some((_, texture)) = textures
                                .iter()
                                .find(|(_, texture)| texture.id() == mesh.texture_id)
                            else {
                                continue;
                            };
                            let rect = mesh.calc_bounds();
                            let well = wells
                                .iter()
                                .find(|well| well.expand(0.5).contains_rect(rect))
                                .unwrap();
                            assert!((rect.center() - well.center()).length() < 0.01);
                            let dimensions = texture.size_vec2();
                            assert!(
                                (rect.aspect_ratio() - dimensions.x / dimensions.y).abs() < 0.001
                            );
                            assert!(well.expand(0.5).contains_rect(shape.clip_rect));
                            for vertex in &mesh.vertices {
                                let expected = (vertex.pos - rect.min) / rect.size();
                                assert!((vertex.uv.to_vec2() - expected).length() < 1e-5);
                            }
                            if rect.intersect(shape.clip_rect).is_positive() {
                                edges[0] |= rect.left() < shape.clip_rect.left();
                                edges[1] |= rect.right() > shape.clip_rect.right();
                            }
                            images += 1;
                        }
                        egui::Shape::Text(text) if text.galley.job.text.starts_with("Camera") => {
                            let bounds = text.galley.rect.translate(text.pos.to_vec2());
                            let well = wells
                                .iter()
                                .find(|well| well.expand(0.5).contains_rect(bounds))
                                .unwrap();
                            assert!(well.expand(0.5).contains_rect(shape.clip_rect));
                            assert!(matches!(
                                text.galley.job.text.as_str(),
                                "Camera" | "Camera preview"
                            ));
                            badges += 1;
                        }
                        egui::Shape::Text(text) if text.galley.job.text.starts_with("photo-") => {
                            assert_eq!(text.galley.rows.len(), 1);
                            assert!(text.galley.job.text.ends_with(".ARW"));
                            captions += 1;
                        }
                        _ => {}
                    }
                }
                assert!(images > 0);
                assert_eq!(badges, images);
                assert_eq!(captions, wells.len());
                assert_eq!(editor.preferences.filmstrip_height, height);
                assert_eq!(editor.document.project, before);
                assert!(editor.ready.is_none() && editor.pending_preview.is_none());
            }
        }
    }
    assert_eq!(
        edges,
        [true, true],
        "exercise JPEG clipping at both scroll edges"
    );
}

#[test]
fn filmstrip_camera_preview_replaces_loading_only_and_cannot_become_selected_ready() {
    let (commands, _) = mpsc::channel();
    let (_, messages) = mpsc::channel();
    let mut editor = Editor::state(commands, messages, Arc::default());
    editor.document = fixture();
    editor.current_source = Some(editor.document.project.order[0]);
    let original = editor.document.project.clone();
    let context = egui::Context::default();
    context.enable_accesskit();
    let image_key = key(&editor.document, editor.document.project.order[0]);
    editor.thumbnails.embedded.entries.push((
        image_key.source.clone(),
        Some(context.load_texture(
            "test-camera",
            egui::ColorImage::filled([8, 4], egui::Color32::RED),
            egui::TextureOptions::LINEAR,
        )),
    ));
    let frame = |editor: &mut Editor| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                ..Default::default()
            },
            |ui| editor.filmstrip(ui),
        );
        output.textures_delta.clear();
        let name = editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .path
            .file_name()
            .unwrap()
            .to_string_lossy();
        let selected = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(name.as_ref()))
            .unwrap();
        assert_eq!(selected.1.toggled(), Some(egui::accesskit::Toggled::True));
        let bounds = selected.1.bounds().unwrap();
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rect)
                if f64::from(rect.rect.left()) == bounds.x0
                    && f64::from(rect.rect.top()) == bounds.y0
                    && f64::from(rect.rect.right()) == bounds.x1
                    && f64::from(rect.rect.bottom()) == bounds.y1
                    && rect.stroke.width > 0.
                    && rect.stroke.color == context.style_of(egui::Theme::Dark).visuals.selection.stroke.color)));
        output
            .shapes
            .into_iter()
            .filter_map(|shape| match shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(
        frame(&mut editor)
            .iter()
            .any(|label| label == "Camera preview")
    );
    assert!(editor.ready.is_none() && editor.pending_preview.is_none());
    assert_eq!(editor.thumbnails.sources.proxy_count(), 0);
    assert_eq!(editor.document.project, original);
    editor.floor = editor.document.project.input_floor.unwrap();
    editor.dmin = Some(glam::DVec3::splat(0.5));
    editor.dmax = Some(glam::DVec3::splat(0.02));
    editor.source = false;
    editor.roll.exposure = 2.;
    editor
        .document
        .project
        .source_mut(image_key.source.id)
        .unwrap()
        .photo = PhotoSettings {
        contrast: 1.2,
        pivot: 0.25,
        contrast_colour_mix: 0.5,
        saturation: 1.3,
        saturation_density: 0.5,
        ..PhotoSettings::default()
    };
    assert!(editor.calibration().is_some());
    assert!(
        frame(&mut editor)
            .iter()
            .any(|label| label == "Camera preview")
    );
    assert_eq!(
        editor
            .thumbnails
            .embedded
            .get(&image_key.source)
            .unwrap()
            .size(),
        [8, 4]
    );
    assert!(editor.ready.is_none() && editor.pending_preview.is_none());
    editor
        .thumbnails
        .sources
        .publish_proxy(
            image_key.source.store_key().unwrap(),
            Arc::new(Proxy {
                width: 8,
                height: 4,
                pixels: vec![[1000, 2000, 3000, 0]; 32],
                matrix: glam::DMat3::IDENTITY,
            }),
        )
        .unwrap();
    assert!(
        frame(&mut editor)
            .iter()
            .any(|label| label == "Camera preview"),
        "keep the camera image until the GPU thumbnail, not just its CPU proxy, is ready"
    );
    let proper = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        GpuImage::new(
            &device,
            &queue,
            8,
            4,
            &[[1000, 2000, 3000, 0]; 32],
            glam::DMat3::IDENTITY,
            editor.target,
        )
        .await
        .unwrap()
    });
    editor
        .thumbnails
        .cache
        .insert(image_key.clone(), Ok(Arc::new(proper)));
    assert!(
        !frame(&mut editor)
            .iter()
            .any(|label| label == "Camera preview")
    );
    assert!(editor.thumbnails.embedded.entries.is_empty());
    assert!(editor.ready.is_none() && editor.pending_preview.is_none());
    editor
        .thumbnails
        .cache
        .insert(image_key, Err("authoritative RAW failure".into()));
    assert!(
        !frame(&mut editor)
            .iter()
            .any(|label| label == "Camera preview")
    );
    assert!(editor.thumbnails.embedded.entries.is_empty());
}
