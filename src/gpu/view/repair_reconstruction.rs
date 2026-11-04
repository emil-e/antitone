use super::*;
use crate::gpu::submission::SubmissionGate;
impl ViewSource {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn reconstruct_repairs(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        base: Arc<Self>,
        repairs: crate::gpu::repairs::Snapshot,
        previous: Option<Arc<Self>>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Arc<Self>>, Error> {
        let scope = base.renderer.evaluation.materializations();
        pollster::block_on(Self::reconstruct_images(
            device,
            queue,
            submissions,
            base,
            repairs,
            previous,
            &scope,
            cancelled,
        ))
    }
    pub fn acquired(&self) -> Arc<crate::gpu::view::acquisition::AcquiredViewRaster> {
        self.data.clone()
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn reconstruct_images(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        base: Arc<Self>,
        repairs: crate::gpu::repairs::Snapshot,
        previous: Option<Arc<Self>>,
        scope: &crate::gpu::evaluation::Materializations,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Arc<Self>>, Error> {
        let acquired = base.acquired();
        let previous_acquired = previous.map(|previous| previous.acquired());
        let result = crate::gpu::view::acquisition::AcquiredViewRaster::reconstruct_repairs(
            device,
            queue,
            submissions,
            acquired.clone(),
            repairs,
            previous_acquired,
            scope,
            cancelled,
        )
        .await?;
        Ok(result.map(|result| {
            if Arc::ptr_eq(&result, &acquired) {
                return base.clone();
            }
            Self::from_acquired(base.kernels.clone(), result)
        }))
    }
}
