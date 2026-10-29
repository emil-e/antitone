//! Rebuild affected physical footprints from originals, never by subtracting a rounded proxy.

use super::*;

impl Acquired {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconstruct_repairs(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        base: Arc<Self>,
        repairs: crate::gpu::repairs::Snapshot,
        previous: Option<Arc<Self>>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Arc<Self>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        if !base.spec.repairs.0.is_empty() {
            return Err(Error::Gpu(
                "repair reconstruction requires an unrepaired neutral base".into(),
            ));
        }
        if repairs.0.is_empty() {
            return Ok(Some(base));
        }
        let previous = previous.filter(|p| p.spec.same_acquisition(&base.spec));
        if let Some(previous) = previous.as_ref().filter(|p| p.spec.repairs == repairs) {
            return Ok(Some(previous.clone()));
        }
        let empty = crate::gpu::repairs::Snapshot::default();
        let old = previous.as_ref().map_or(&empty, |p| &p.spec.repairs);
        let changed = repairs
            .0
            .iter()
            .filter(|p| !old.0.iter().any(|q| Arc::ptr_eq(p, q)))
            .chain(
                old.0
                    .iter()
                    .filter(|p| !repairs.0.iter().any(|q| Arc::ptr_eq(p, q))),
            );
        let changed = crate::gpu::repairs::Snapshot(changed.cloned().collect());
        let frame = base.spec.input_frame()?;
        let windows = base.spec.plan.affected_windows(
            changed.0.iter().map(|patch| {
                let support = patch.evaluated.support();
                let origin = support.origin();
                let size = support.size();
                [origin[0], origin[1], size[0], size[1]].into()
            }),
            Default::default(),
        )?;
        let pixels = float_buffer(device, frame.output)?;
        let initial = previous.as_ref().unwrap_or(&base);
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&initial.pixels, 0, &pixels, 0, pixels.size());
        if base
            .renderer
            .evaluation
            .submit(queue, submissions, encoder, &cancelled)?
            .is_none()
        {
            return Ok(None);
        }
        for (origin, size) in windows {
            if cancelled() {
                return Ok(None);
            }
            let region = base.spec.plan.window_frame((origin, size))?;
            // The repair window is already on the acquired input grid; it needs no new stage halo.
            let spec = base
                .spec
                .clone()
                .with_repairs(repairs.clone())
                .with_pipeline(Default::default())?
                .with_frame(region)?;
            let Some(mut rebuilt) = Self::acquire_many(
                device,
                queue,
                submissions,
                base.renderer.clone(),
                base.kernels.clone(),
                scan,
                field.clone(),
                vec![spec],
                &cancelled,
            )?
            else {
                return Ok(None);
            };
            let rebuilt = rebuilt.remove(0);
            let mut encoder = device.create_command_encoder(&Default::default());
            // Restore changed footprints first; erased/deleted support returns to exact base bits.
            crate::gpu::evaluation::acquire::repair_copy(
                &base.renderer.evaluation,
                device,
                &mut encoder,
                &pixels,
                (&base.pixels, ([0, 0], frame.output)),
                frame,
                (origin, size),
                &changed,
            )?;
            crate::gpu::evaluation::acquire::repair_copy(
                &base.renderer.evaluation,
                device,
                &mut encoder,
                &pixels,
                (&rebuilt.pixels, (origin, size)),
                frame,
                (origin, size),
                &repairs,
            )?;
            if base
                .renderer
                .evaluation
                .submit(queue, submissions, encoder, &cancelled)?
                .is_none()
            {
                return Ok(None);
            }
        }
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(Arc::new(Self {
            spec: base.spec.clone().with_repairs(repairs),
            pixels,
            renderer: base.renderer.clone(),
            kernels: base.kernels.clone(),
        })))
    }
}
