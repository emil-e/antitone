//! Rebuild affected physical footprints from originals, never by subtracting a rounded proxy.
use super::*;

impl ViewSource {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconstruct_repairs(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        base: Arc<Self>,
        repairs: super::super::repairs::Snapshot,
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
        let previous =
            previous.filter(|p| p.spec.same_source(&base.spec) && p.spec.frame == base.spec.frame);
        if let Some(previous) = previous.as_ref().filter(|p| p.spec.repairs == repairs) {
            return Ok(Some(previous.clone()));
        }
        let empty = super::super::repairs::Snapshot::default();
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
        let changed = super::super::repairs::Snapshot(changed.cloned().collect());
        let frame = base.spec.frame;
        let bounds = frame.bounds()?;
        let affine = frame.affine()?;
        let mut windows = Vec::new();
        for patch in &changed.0 {
            let support = patch.evaluated.support();
            let origin = support.origin();
            let size = support.size();
            let lo = [origin[0].max(bounds.x), origin[1].max(bounds.y)];
            let hi = [
                (origin[0] + size[0]).min(bounds.x + bounds.width),
                (origin[1] + size[1]).min(bounds.y + bounds.height),
            ];
            if (0..2).any(|c| lo[c] >= hi[c]) {
                continue;
            }
            windows.extend(crate::resample::windows::bounded(
                crate::resample::windows::affine_output(
                    affine,
                    frame.output,
                    [lo[0] - bounds.x, lo[1] - bounds.y],
                    [hi[0] - lo[0], hi[1] - lo[1]],
                ),
            ));
        }
        windows.sort_unstable();
        windows.dedup();
        let pixels = float_buffer(device, frame.output)?;
        let initial = previous.as_ref().unwrap_or(&base);
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&initial.pixels, 0, &pixels, 0, pixels.size());
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submissions.submit(queue, [encoder.finish()])),
                timeout: None,
            })
            .map_err(|e| Error::Gpu(e.to_string()))?;
        for (origin, size) in windows {
            if cancelled() {
                return Ok(None);
            }
            let point = |offset: [u32; 2]| {
                crate::geometry::FramePoint(std::array::from_fn(|c| {
                    frame.min.0[c]
                        + f64::from(offset[c]) * (frame.max.0[c] - frame.min.0[c])
                            / f64::from(frame.output[c])
                }))
            };
            let region = FrameRaster::new(
                frame.composition,
                point(origin),
                point([origin[0] + size[0], origin[1] + size[1]]),
                size,
            )?;
            let mut spec = base.spec.clone().with_repairs(repairs.clone());
            spec.frame = region;
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
            base.renderer.repairs.copy_regions(
                device,
                &mut encoder,
                &pixels,
                (&base.pixels, ([0, 0], frame.output)),
                frame,
                (origin, size),
                &changed,
            )?;
            base.renderer.repairs.copy_regions(
                device,
                &mut encoder,
                &pixels,
                (&rebuilt.pixels, (origin, size)),
                frame,
                (origin, size),
                &repairs,
            )?;
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submissions.submit(queue, [encoder.finish()])),
                    timeout: None,
                })
                .map_err(|e| Error::Gpu(e.to_string()))?;
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
