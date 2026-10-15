use super::*;

pub(super) struct Composite {
    detail: Arc<RenderedView>,
    underlay: Arc<RenderedView>,
    group: wgpu::BindGroup,
}
impl RenderedView {
    pub(crate) fn composite(
        device: &wgpu::Device,
        detail: Arc<Self>,
        underlay: Arc<Self>,
        frame: FrameRaster,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        // Never recursively reconstruct an old composite, mix revisions, or repaint a reference
        // as though it were neutral working support. Only immutable matching colour parents.
        if detail.composite.is_some()
            || underlay.composite.is_some()
            || detail.graded.is_none()
            || underlay.graded.is_none()
            || !detail.spec.same_source(&underlay.spec)
            || frame.composition.area() != detail.spec.area
            || detail.processing != underlay.processing
            || detail.target != underlay.target
        {
            return Err(Error::Gpu(
                "physical-view underlay and detail identities differ".into(),
            ));
        }
        if detail.spec.frame == frame {
            return Ok(detail);
        }
        if let Some(old) = previous.filter(|p| {
            p.spec.frame == frame
                && p.composite.as_ref().is_some_and(|c| {
                    Arc::ptr_eq(&c.detail, &detail) && Arc::ptr_eq(&c.underlay, &underlay)
                })
        }) {
            return Ok(old.clone());
        }
        // Navigation must not accumulate full-display targets behind the GPU. Keep the last
        // accepted mapping until its submission completes, then compose the latest requested
        // frame. Its completion wakes the UI; callers must not relabel the held frame as current.
        if let Some(old) = previous.filter(|p| {
            p.composite.is_some()
                && p.in_flight()
                && p.spec.authority == detail.spec.authority
                && p.target == detail.target
        }) {
            return Ok(old.clone());
        }
        let source = &detail.graded.as_ref().unwrap().source;
        let image = Arc::new(pollster::block_on(GpuImage::new_rendered_shared(
            &source.renderer,
            device,
            frame.output[0],
            frame.output[1],
            detail.target,
        ))?);
        let maps = [
            mapping(detail.spec.frame, frame),
            mapping(underlay.spec.frame, frame),
        ];
        let mut bytes = bytemuck::cast_slice(&[frame.output[0], frame.output[1], 0, 0]).to_vec();
        for m in maps {
            bytes.extend_from_slice(bytemuck::cast_slice(&[
                m.origin[0],
                m.origin[1],
                m.x_axis[0],
                m.x_axis[1],
                m.y_axis[0],
                m.y_axis[1],
                0.,
                0.,
            ]));
        }
        let parameters = uniform(device, &bytes);
        let textures =
            [&detail, &underlay].map(|p| p.image.linear.create_view(&Default::default()));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("coherent detail and coarse-coverage composition"),
            layout: &source.kernels.compose.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&textures[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&textures[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: parameters.as_entire_binding(),
                },
            ],
        });
        let mut spec = detail.spec.clone();
        spec.frame = frame;
        Ok(Arc::new(Self {
            image,
            spec,
            processing: detail.processing.clone(),
            target: detail.target,
            graded: None,
            group: None,
            composite: Some(Composite {
                detail,
                underlay,
                group,
            }),
            submitted: Mutex::new(false),
            complete: Arc::new(AtomicBool::new(false)),
        }))
    }
}
impl Composite {
    pub(super) fn ensure_parents(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gate: &SubmissionGate,
        context: &egui::Context,
    ) {
        self.detail.ensure_submitted(device, queue, gate, context);
        self.underlay.ensure_submitted(device, queue, gate, context);
    }
    pub(super) fn encode(&self, encoder: &mut wgpu::CommandEncoder, image: &GpuImage) {
        let view = image.linear.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("accepted current-frame detail/underlay composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.detail.graded.as_ref().unwrap().source.kernels.compose);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

pub(super) fn pipeline(
    device: &wgpu::Device,
    label: &'static str,
    shader: &'static str,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba32Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}
pub(super) fn mapping(old: FrameRaster, frame: FrameRaster) -> TextureMapping {
    let origin = old.source_point([0., 0.]).0;
    let right = old.source_point([f64::from(old.output[0]), 0.]).0;
    let bottom = old.source_point([0., f64::from(old.output[1])]).0;
    let dx = [right[0] - origin[0], right[1] - origin[1]];
    let dy = [bottom[0] - origin[0], bottom[1] - origin[1]];
    let det = dx[0] * dy[1] - dx[1] * dy[0];
    let uv = |p: [f64; 2]| {
        let p = frame.source_point(p).0;
        let p = [p[0] - origin[0], p[1] - origin[1]];
        [
            (p[0] * dy[1] - p[1] * dy[0]) / det,
            (dx[0] * p[1] - dx[1] * p[0]) / det,
        ]
    };
    let start = uv([0., 0.]);
    let x = uv([f64::from(frame.output[0]), 0.]);
    let y = uv([0., f64::from(frame.output[1])]);
    TextureMapping {
        origin: start.map(|v| v as f32),
        x_axis: std::array::from_fn(|c| (x[c] - start[c]) as f32),
        y_axis: std::array::from_fn(|c| (y[c] - start[c]) as f32),
    }
}
