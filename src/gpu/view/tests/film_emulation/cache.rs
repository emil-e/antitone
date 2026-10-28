use super::*;

fn candidate(
    device: &wgpu::Device,
    source: Arc<ViewSource>,
    film: FilmEmulationSettings,
    photo: AdjustmentLayerSettings,
    previous: Option<&Arc<RenderedView>>,
) -> Arc<RenderedView> {
    let snapshot =
        AdjustmentLayerSnapshot::new(vec![crate::adjustment_layers::AdjustmentLayer::global(
            photo,
        )])
        .unwrap();
    let grade = Arc::new(
        GpuPipeline::new(
            device,
            InputSettings::Digital,
            RollSettings::default(),
            snapshot.clone(),
            source.spec.matrix,
            source.spec.area,
        )
        .unwrap(),
    );
    let input = ProcessingInput::from(grade)
        .with_film_emulation(film)
        .with_frame_size(source.spec.frame().composition.crop().size)
        .with_film_seed(17);
    RenderedView::new(device, source, &input, OutputTarget::Srgb, previous).unwrap()
}

fn field(view: &RenderedView) -> &Arc<crate::gpu::grain::Field> {
    view.graded.as_ref().unwrap().stages.field().unwrap()
}

#[test]
fn film_emulation_gpu_encoded_field_survives_owner_retirement_before_submission() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let view = probe_view(&device, &queue, &renderer, &kernels, [2., 1.], true, 17);
        let weak_view = Arc::downgrade(&view);
        let weak_field = Arc::downgrade(field(&view));
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("deferred film result"),
            size: (N * N * 16) as u64,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let graded = view.graded.as_ref().unwrap();
            kernels.grade(&device, &mut encoder, graded);
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
fn film_emulation_gpu_reuses_grain_across_amount_colour_and_grade_edits() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
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
        let source = base.graded.as_ref().unwrap().source.clone();
        let settings = FilmEmulationSettings {
            enabled: true,
            size: 2. / 6000. * 100.,
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
            FilmEmulationSettings {
                amount: 0.,
                ..settings
            },
            AdjustmentLayerSettings::default(),
            Some(&base),
        );
        zero.ensure_submitted(
            &device,
            &queue,
            &SubmissionGate::default(),
            &egui::Context::default(),
        );
        assert!(Arc::ptr_eq(field(&base), field(&zero)));
        assert!(
            !field(&zero).ready(),
            "zero Amount must not compute or mark an uninitialized field ready"
        );
        let edited = candidate(&device, source.clone(), settings, photo, Some(&zero));
        assert!(Arc::ptr_eq(field(&base), field(&edited)));
        assert!(!field(&base).ready());

        // Encoding alone is not initialization, including when a different photographic view
        // shares the field. A deferred/abandoned UI encoder must not poison later publication.
        let mut abandoned = device.create_command_encoder(&Default::default());
        kernels.grade(&device, &mut abandoned, base.graded.as_ref().unwrap());
        drop(abandoned);
        assert!(!field(&edited).ready());
        edited.ensure_submitted(
            &device,
            &queue,
            &SubmissionGate::default(),
            &egui::Context::default(),
        );
        assert!(field(&base).ready());
        let cached = read(&device, &queue, &edited.graded.as_ref().unwrap().pixels);
        let generated = read(&device, &queue, &field(&edited).pixels);
        let variance = settings
            .parameters([6000; 2])
            .unwrap()
            .unwrap()
            .channel_variance(1.);
        // Red/blue are flat; green carries the impulse. Verify refreshed photographic
        // and colour uniforms per pixel, independently of a small crop's random mean.
        let graded = edited.graded.as_ref().unwrap();
        let input_size = graded.stages.raster.size();
        let output_size = graded.plan.output().output;
        let offset = graded.plan.output_offset().unwrap();
        for (index, pixel) in cached.iter().enumerate() {
            let point = [
                index as u32 % output_size[0] + offset[0],
                index as u32 / output_size[0] + offset[1],
            ];
            let grain = generated[(point[1] * input_size[0] + point[0]) as usize];
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
        let warm_settings = FilmEmulationSettings {
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
        assert!(Arc::ptr_eq(field(&edited), field(&warm)));
        drop(base);
        drop(edited);
        warm.ensure_submitted(
            &device,
            &queue,
            &SubmissionGate::default(),
            &egui::Context::default(),
        );
        let warm_pixels = read(&device, &queue, &warm.graded.as_ref().unwrap().pixels);
        let cold = candidate(
            &device,
            source.clone(),
            warm_settings,
            AdjustmentLayerSettings::default(),
            None,
        );
        assert!(!Arc::ptr_eq(field(&warm), field(&cold)));
        cold.ensure_submitted(
            &device,
            &queue,
            &SubmissionGate::default(),
            &egui::Context::default(),
        );
        let cold_pixels = read(&device, &queue, &cold.graded.as_ref().unwrap().pixels);
        assert_eq!(warm_pixels, cold_pixels);
        assert_ne!(
            warm_pixels, cached,
            "field reuse must not reuse the previous photographic output"
        );

        let resized = candidate(
            &device,
            source.clone(),
            FilmEmulationSettings {
                size: settings.size * 1.5,
                ..settings
            },
            photo,
            Some(&warm),
        );
        assert!(!Arc::ptr_eq(field(&warm), field(&resized)));
        let mut other_spec = source.spec.clone();
        // Publication rights do not change the source-anchored field's geometry/model/seed.
        other_spec.authority += 65536;
        let other_source = Arc::new(ViewSource {
            spec: other_spec,
            pixels: source.pixels.clone(),
            renderer: renderer.clone(),
            kernels: kernels.clone(),
        });
        let other = candidate(
            &device,
            other_source,
            warm_settings,
            AdjustmentLayerSettings::default(),
            Some(&warm),
        );
        assert!(Arc::ptr_eq(field(&warm), field(&other)));
        other.ensure_submitted(
            &device,
            &queue,
            &SubmissionGate::default(),
            &egui::Context::default(),
        );
        assert_eq!(
            read(&device, &queue, &field(&warm).pixels),
            read(&device, &queue, &field(&other).pixels),
            "reacquiring a source must retain its grain realization"
        );

        let mut shifted_spec = source.spec.clone();
        let mut shifted_frame = shifted_spec.frame();
        shifted_frame.min.0 = shifted_frame.min.0.map(|x| x + 8.);
        shifted_frame.max.0 = shifted_frame.max.0.map(|x| x + 8.);
        shifted_spec = shifted_spec.with_frame(shifted_frame).unwrap();
        let shifted_source = Arc::new(ViewSource {
            spec: shifted_spec,
            pixels: source.pixels.clone(),
            renderer,
            kernels,
        });
        let shifted = candidate(
            &device,
            shifted_source,
            warm_settings,
            AdjustmentLayerSettings::default(),
            Some(&warm),
        );
        assert!(!Arc::ptr_eq(field(&warm), field(&shifted)));
        shifted.ensure_submitted(
            &device,
            &queue,
            &SubmissionGate::default(),
            &egui::Context::default(),
        );
        let a = read(&device, &queue, &field(&warm).pixels);
        let b = read(&device, &queue, &field(&shifted).pixels);
        let size = warm
            .graded
            .as_ref()
            .unwrap()
            .stages
            .raster
            .size()
            .map(|n| n as usize);
        for y in 0..size[1] - 8 {
            for x in 0..size[0] - 8 {
                assert_eq!(a[(y + 8) * size[0] + x + 8], b[y * size[0] + x]);
            }
        }
    });
}
