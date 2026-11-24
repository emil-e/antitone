use super::*;
use crate::gpu::test_inputs::AuthoredPhotograph;

fn candidate(
    device: &wgpu::Device,
    source: Arc<ViewImage>,
    film: GrainSettings,
    photo: AdjustmentLayerSettings,
    previous: Option<&Arc<RenderedView>>,
) -> Arc<RenderedView> {
    let snapshot =
        AdjustmentLayerSnapshot::new(vec![crate::adjustment_layers::AdjustmentLayer::global(
            photo,
        )])
        .unwrap();
    let grade = Arc::new(
        AuthoredPhotograph::new(
            &device.limits(),
            InputSettings::Digital,
            RollSettings::default(),
            snapshot.clone(),
            source.spec.matrix(),
            source.spec.area,
        )
        .unwrap(),
    );
    let input = AuthoredPhotograph::from(grade).with_definition(
        crate::photographic::graph::assembly::Assembly::emulation(
            film,
            Default::default(),
            source.spec.frame().composition.crop().size,
            17,
        )
        .unwrap(),
    );
    RenderedView::new(device, source, &input, OutputTarget::Srgb, previous).unwrap()
}

fn field(view: &RenderedView) -> Arc<crate::gpu::evaluation::RetainedBuffer> {
    cached_field(view).unwrap()
}

#[test]
fn halation_strength_refreshes_pixels_but_reuses_completed_grain_at_fixed_support() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        renderer
            .evaluation()
            .materializations()
            .set_test_pressure(crate::sys::MemoryPressure::Normal);
        let base = configured_probe(
            &device,
            &renderer,
            &kernels,
            ProbeConfig {
                reserve_scale: Some(4.),
                impulse: true,
                ..Default::default()
            },
            17,
        );
        let source = base.product.as_ref().unwrap().fixture_input();
        let make = |strength, previous: &Arc<RenderedView>| {
            let input = AuthoredPhotograph::new(
                &device.limits(),
                InputSettings::Digital,
                RollSettings::default(),
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                source.spec.matrix(),
                source.spec.area,
            )
            .unwrap()
            .with_definition(
                crate::photographic::graph::assembly::Assembly::emulation(
                    GrainSettings {
                        enabled: true,
                        size: 2. / 6000. * 100.,
                        image_sigma: 2. / 6000. * 100.,
                        ..Default::default()
                    },
                    crate::halation::HalationSettings {
                        enabled: true,
                        size: 0.02,
                        strength,
                    },
                    source.spec.frame().composition.crop().size,
                    17,
                )
                .unwrap(),
            );
            RenderedView::new(
                &device,
                source.clone(),
                &input,
                OutputTarget::Srgb,
                Some(previous),
            )
            .unwrap()
        };
        let first = make(0.2, &base);
        let first_field = field(&first);
        first.ensure_submitted(&device, &queue, &egui::Context::default());
        let pixels = read(&device, &queue, &first.product.as_ref().unwrap().pixels);
        let noise = read(&device, &queue, &first_field.pixels);
        assert!(first_field.ready());
        let first_buffer = first_field.pixels.clone();
        drop(first_field);
        let changed = make(0.8, &first);
        let changed_field = field(&changed);
        assert_eq!(first_buffer, changed_field.pixels);
        assert!(changed_field.ready());
        changed.ensure_submitted(&device, &queue, &egui::Context::default());
        assert_eq!(noise, read(&device, &queue, &changed_field.pixels));
        let rendered = read(&device, &queue, &changed.product.as_ref().unwrap().pixels);
        assert!(
            pixels
                .iter()
                .zip(rendered)
                .any(|(a, b)| (a[1] - b[1]).abs() > 1e-5),
            "nonconstant fixture must expose the halation edit"
        );
    });
}

#[test]
fn grain_gpu_encoded_field_survives_owner_retirement_before_submission() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        renderer
            .evaluation()
            .materializations()
            .set_test_pressure(crate::sys::MemoryPressure::Normal);
        let view = probe_view(&device, &queue, &renderer, &kernels, [2., 1.], true, 17);
        let weak_view = Arc::downgrade(&view);
        let weak_field = Arc::downgrade(&field(&view));
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("deferred film result"),
            size: (N * N * 16) as u64,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let graded = view.product.as_ref().unwrap();
            graded.encode(&device, &mut encoder);
            encoder.copy_buffer_to_buffer(&graded.pixels, 0, &output, 0, output.size());
        }
        drop(view);
        assert!(weak_view.upgrade().is_none() && weak_field.upgrade().is_none());
        queue.submit([encoder.finish()]);
        let pixels = read(&device, &queue, &output);
        assert!(
            pixels
                .iter()
                .all(|p| p[3] == 8. && p[..3].iter().all(|v| v.is_finite()))
        );
        let (low, high) = pixels
            .iter()
            .map(|p| p[0])
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), x| {
                (lo.min(x), hi.max(x))
            });
        assert!(
            high - low > 0.0001,
            "deferred execution must generate and consume the grain field"
        );
    });
}

#[test]
fn grain_gpu_reuses_grain_across_amount_colour_and_grade_edits() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        renderer
            .evaluation()
            .materializations()
            .set_test_pressure(crate::sys::MemoryPressure::Normal);
        // Keep acquisition layout fixed while changing Size, so the field-key assertion
        // cannot pass merely because insufficient support forced a different raster.
        let base = configured_probe(
            &device,
            &renderer,
            &kernels,
            ProbeConfig {
                reserve_scale: Some(3.),
                impulse: true,
                ..Default::default()
            },
            17,
        );
        let source = base.product.as_ref().unwrap().fixture_input();
        let settings = GrainSettings {
            enabled: true,
            size: 2. / 6000. * 100.,
            image_sigma: 2. / 6000. * 100.,
            amount: 0.1,
            colour: 0.8,
        };
        let photo = AdjustmentLayerSettings {
            exposure: 0.5,
            ..Default::default()
        };
        let zero = candidate(
            &device,
            source.clone(),
            GrainSettings {
                amount: 0.,
                ..settings
            },
            AdjustmentLayerSettings::default(),
            Some(&base),
        );
        let cold_zero = candidate(
            &device,
            source.clone(),
            GrainSettings {
                amount: 0.,
                ..settings
            },
            AdjustmentLayerSettings::default(),
            None,
        );
        assert!(
            cached_field(&cold_zero).is_none(),
            "omitted grain does not allocate a field; warm reuse may retain an existing value"
        );
        drop(cold_zero);
        assert!(cached_field(&zero).is_none());
        zero.ensure_submitted(&device, &queue, &egui::Context::default());
        assert!(
            !field(&base).ready(),
            "zero Amount must not compute or mark an uninitialized field ready"
        );
        let edited = candidate(&device, source.clone(), settings, photo, Some(&zero));
        assert_eq!(field(&base).pixels, field(&edited).pixels);
        assert!(!field(&base).ready());

        // Encoding alone is not initialization, including when a different photographic view
        // shares the field. A deferred/abandoned UI encoder must not poison later publication.
        let mut abandoned = device.create_command_encoder(&Default::default());
        base.product
            .as_ref()
            .unwrap()
            .encode(&device, &mut abandoned);
        drop(abandoned);
        assert!(!field(&edited).ready());
        let edited_field = field(&edited).pixels.clone();
        edited.ensure_submitted(&device, &queue, &egui::Context::default());
        assert!(field(&base).ready());
        let cached = read(&device, &queue, &edited.product.as_ref().unwrap().pixels);
        let generated = read(&device, &queue, &edited_field);
        let variance = settings
            .parameters([6000; 2])
            .unwrap()
            .unwrap()
            .channel_variance(1.);
        // Red/blue are flat; green carries the impulse. Verify refreshed photographic
        // and colour uniforms per pixel, independently of a small crop's random mean.
        assert_eq!(
            generated.len(),
            cached.len(),
            "grain is integrated on presentation cells"
        );
        for (index, pixel) in cached.iter().enumerate() {
            let grain = generated[index];
            for c in [0, 2] {
                let g = grain[1] as f64 + 2. * settings.colour * (grain[c] - grain[1]) as f64;
                let expected = 0.18
                    * 2_f64.sqrt()
                    * (settings.amount * g - 0.5 * settings.amount.powi(2) * variance[c]).exp();
                assert!((pixel[c] as f64 / expected - 1.).abs() < 0.000003);
            }
        }
        assert!(generated.iter().flatten().all(|g| g.is_finite()));
        assert!(generated.iter().any(|g| (g[0] - g[1]).abs() > 0.1));
        assert!(generated.iter().any(|g| g[0].abs() > 0.1));

        drop(zero);
        let warm_settings = GrainSettings {
            amount: 0.075,
            colour: 0.3,
            ..settings
        };
        let warm = candidate(
            &device,
            source.clone(),
            warm_settings,
            AdjustmentLayerSettings::default(),
            Some(&edited),
        );
        let warm_field = field(&warm).pixels.clone();
        assert_eq!(edited_field, warm_field);
        drop(base);
        drop(edited);
        warm.ensure_submitted(&device, &queue, &egui::Context::default());
        let warm_pixels = read(&device, &queue, &warm.product.as_ref().unwrap().pixels);
        let cold_renderer = crate::render_fixture::RenderFixture::new(&device, &queue)
            .await
            .unwrap();
        cold_renderer
            .evaluation()
            .materializations()
            .set_test_pressure(crate::sys::MemoryPressure::Normal);
        let cold_source = Arc::new(ViewImage::from_neutral_samples(
            source.spec.clone(),
            cold_renderer,
            kernels.clone(),
            (*source.pixels).clone(),
        ));
        let cold = candidate(
            &device,
            cold_source,
            warm_settings,
            AdjustmentLayerSettings::default(),
            None,
        );
        assert_ne!(warm_field, field(&cold).pixels);
        cold.ensure_submitted(&device, &queue, &egui::Context::default());
        let cold_pixels = read(&device, &queue, &cold.product.as_ref().unwrap().pixels);
        assert_eq!(warm_pixels, cold_pixels);
        assert_ne!(
            warm_pixels, cached,
            "field reuse must not reuse the previous photographic output"
        );

        let softer = candidate(
            &device,
            source.clone(),
            GrainSettings {
                image_sigma: warm_settings.image_sigma * 1.2,
                ..warm_settings
            },
            AdjustmentLayerSettings::default(),
            Some(&warm),
        );
        assert_eq!(
            warm_field,
            field(&softer).pixels,
            "independent image softness reuses the selected field"
        );
        softer.ensure_submitted(&device, &queue, &egui::Context::default());
        let softer_pixels = read(&device, &queue, &softer.product.as_ref().unwrap().pixels);
        assert!(
            warm_pixels
                .iter()
                .zip(softer_pixels)
                .any(|(a, b)| (a[1] - b[1]).abs() > 1e-5),
            "the impulse must expose omitted image-sigma edits"
        );

        let resized = candidate(
            &device,
            source.clone(),
            GrainSettings {
                size: settings.size * 1.5,
                image_sigma: settings.size * 1.5,
                ..settings
            },
            photo,
            Some(&warm),
        );
        assert_ne!(warm_field, field(&resized).pixels);
        let other_source = independent_image(&source);
        assert!(!Arc::ptr_eq(&source, &other_source));
        let other = candidate(&device, other_source, warm_settings, photo, Some(&warm));
        let other_field = field(&other).pixels.clone();
        assert_eq!(warm_field, other_field);
        other.ensure_submitted(&device, &queue, &egui::Context::default());
        assert_eq!(
            read(&device, &queue, &warm_field),
            read(&device, &queue, &other_field),
            "reacquiring a source must retain its grain realization"
        );

        let mut shifted_spec = source.spec.clone();
        let mut shifted_frame = shifted_spec.frame();
        shifted_frame.min.0 = shifted_frame.min.0.map(|x| x + 8.);
        shifted_frame.max.0 = shifted_frame.max.0.map(|x| x + 8.);
        shifted_spec = shifted_spec.with_frame(shifted_frame).unwrap();
        let shifted_source = Arc::new(ViewImage::from_neutral_samples(
            shifted_spec,
            renderer,
            kernels,
            (*source.pixels).clone(),
        ));
        let shifted = candidate(
            &device,
            shifted_source,
            warm_settings,
            AdjustmentLayerSettings::default(),
            Some(&warm),
        );
        let shifted_field = field(&shifted).pixels.clone();
        assert_ne!(warm_field, shifted_field);
        shifted.ensure_submitted(&device, &queue, &egui::Context::default());
        let a = read(&device, &queue, &warm_field);
        let b = read(&device, &queue, &shifted_field);
        let size = warm.spec.frame().output.map(|n| n as usize);
        for y in 0..size[1] - 8 {
            for x in 0..size[0] - 8 {
                assert_eq!(a[(y + 8) * size[0] + x + 8], b[y * size[0] + x]);
            }
        }
    });
}

#[test]
fn grain_retention_limits_and_pressure_preserve_rendered_pixels() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        renderer
            .evaluation()
            .materializations()
            .set_test_pressure(crate::sys::MemoryPressure::Normal);
        let baseline = configured_probe(&device, &renderer, &kernels, ProbeConfig::default(), 17);
        let baseline_field = field(&baseline);
        baseline.ensure_submitted(&device, &queue, &egui::Context::default());
        let expected = read(&device, &queue, &baseline.product.as_ref().unwrap().pixels);
        assert!(baseline_field.ready());
        let no_grain = configured_probe(
            &device,
            &renderer,
            &kernels,
            ProbeConfig {
                amount: 0.,
                ..Default::default()
            },
            17,
        );
        no_grain.ensure_submitted(&device, &queue, &egui::Context::default());
        let control = read(&device, &queue, &no_grain.product.as_ref().unwrap().pixels);
        assert!(
            expected
                .iter()
                .zip(control)
                .any(|(a, b)| (a[0] - b[0]).abs() > 1e-4),
            "fixture must detect omitted grain"
        );
        for (pressure, bytes) in [
            (crate::sys::MemoryPressure::Normal, 0),
            (crate::sys::MemoryPressure::Normal, 1),
            (crate::sys::MemoryPressure::Warning, 256 << 20),
            (crate::sys::MemoryPressure::Critical, 256 << 20),
        ] {
            let uncached_renderer = crate::render_fixture::RenderFixture::new_with_cache_policy(
                &device,
                &queue,
                crate::gpu::evaluation::CachePolicy {
                    generated_bytes: bytes,
                },
            )
            .await
            .unwrap();
            uncached_renderer
                .evaluation()
                .materializations()
                .set_test_pressure(pressure);
            for _ in 0..2 {
                let view = configured_probe(
                    &device,
                    &uncached_renderer,
                    &kernels,
                    ProbeConfig::default(),
                    17,
                );
                assert!(
                    cached_field(&view).is_some(),
                    "a live recording owns its required field independently of optional retention"
                );
                view.ensure_submitted(&device, &queue, &egui::Context::default());
                assert_eq!(
                    read(&device, &queue, &view.product.as_ref().unwrap().pixels),
                    expected
                );
                assert_eq!(uncached_renderer.evaluation().retained_bytes(), 0);
            }
        }
    });
}
