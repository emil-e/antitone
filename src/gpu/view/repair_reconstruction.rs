use super::*;
impl ViewSource {
    pub(super) fn acquired(&self) -> Arc<crate::gpu::execution::footprints::Acquired> {
        Arc::new(crate::gpu::execution::footprints::Acquired {
            spec: self.spec.request.clone(),
            renderer: self.renderer.clone(),
            kernels: self.kernels.acquisition.clone(),
            pixels: self.pixels.clone(),
        })
    }
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
        let acquired = base.acquired();
        let previous_acquired = previous.as_ref().map(|p| p.acquired());
        let result = crate::gpu::execution::footprints::Acquired::reconstruct_repairs(
            device,
            queue,
            submissions,
            scan,
            field,
            acquired.clone(),
            repairs,
            previous_acquired.clone(),
            cancelled,
        )?;
        Ok(result.map(|result| {
            if Arc::ptr_eq(&result, &acquired) {
                return base.clone();
            }
            if let Some(previous_acquired) = &previous_acquired
                && Arc::ptr_eq(&result, previous_acquired)
            {
                return previous.as_ref().unwrap().published(base.spec.authority);
            }
            Self::from_acquired(base.spec.authority, base.kernels.clone(), result)
        }))
    }
}
