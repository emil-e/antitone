//! Admitted active source storage, independently of optional image retention.
use super::CellRegion;
use super::*;

#[cfg(test)]
mod tests;

pub(crate) struct ActiveInput {
    images: Vec<Image<ImageLayout>>,
}
impl ActiveInput {
    pub(crate) fn matches(&self, source: &ImageNode) -> bool {
        self.images
            .first()
            .is_some_and(|image| Some(&image.identity) == source.identity())
    }
}

fn allowance(ram: Option<u64>, gpu: crate::sys::GpuMemory) -> u64 {
    // Static capacity bounds this optimization; it is not an estimate of live headroom.
    let bytes = match gpu {
        crate::sys::GpuMemory::Unified => ram.filter(|v| *v > 0).map(|bytes| bytes / 32),
        crate::sys::GpuMemory::Discrete { dedicated_bytes } => {
            dedicated_bytes.filter(|v| *v > 0).map(|bytes| bytes / 8)
        }
        crate::sys::GpuMemory::Unknown => None,
    }
    .unwrap_or(256 << 20);
    bytes.min(512 << 20)
}
pub(super) fn capacity(device: &wgpu::Device) -> u64 {
    allowance(
        Some(crate::sys::physical_memory()),
        crate::sys::gpu_memory(Some(device)),
    )
}

fn parts(bounds: CellRegion, limits: &wgpu::Limits) -> Option<Vec<CellRegion>> {
    let edge = limits.max_texture_dimension_2d;
    if edge == 0 {
        return None;
    }
    let columns = bounds.width.div_ceil(edge);
    let rows = bounds.height.div_ceil(edge);
    if u64::from(columns) * u64::from(rows) > super::materialized::MAX_PARTS as u64 {
        return None;
    }
    Some(
        (0..rows)
            .flat_map(|row| {
                (0..columns).map(move |column| {
                    let offset = |length: u32, index: u32, parts: u32| {
                        (u64::from(length) * u64::from(index) / u64::from(parts)) as i32
                    };
                    let x = bounds.x + offset(bounds.width, column, columns);
                    let y = bounds.y + offset(bounds.height, row, rows);
                    [
                        x,
                        y,
                        bounds.x + offset(bounds.width, column + 1, columns) - x,
                        bounds.y + offset(bounds.height, row + 1, rows) - y,
                    ]
                    .into()
                })
            })
            .collect(),
    )
}

impl Context {
    /// Payload reserved by active source inputs, including submitted consumers after retirement.
    pub fn active_input_bytes(&self) -> u64 {
        self.active_inputs.bytes()
    }

    pub(crate) async fn prepare_input(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::SubmissionGate,
        scope: &Materializations,
        source: Arc<ImageNode>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<ActiveInput>, Error> {
        if cancelled() {
            return Ok(None);
        }
        let acquired = source.source().ok_or(Error::InvalidParameter(
            "active input requires acquired source samples",
        ))?;
        let format = acquired.storage_format();
        let domain = source.domain();
        let bounds = domain.finite()?.validate()?;
        let bytes = u64::from(bounds.width)
            .checked_mul(u64::from(bounds.height))
            .and_then(|cells| cells.checked_mul(format.pixel_bytes()))
            .ok_or(Error::InvalidParameter(
                "active input exceeds storage range",
            ))?;
        let Some(regions) = parts(bounds, &self.device().limits()) else {
            return Ok(None);
        };
        let Some(admission) = self.active_inputs.acquire(bytes) else {
            return Ok(None);
        };
        let admission = Arc::new(admission);
        let mut images = Vec::new();
        for bounds in regions {
            if cancelled() {
                return Ok(None);
            }
            let evaluation = self.request(
                source.clone(),
                Region {
                    bounds,
                    grid: domain.grid,
                },
                Budget {
                    tile: [512, 128],
                    scratch_bytes: 32 << 20,
                },
            )?;
            let mut pending = Box::pin(self.materialize_into(
                queue,
                gate,
                scope,
                evaluation,
                (format, ImageBacking::Texture),
                Some(admission.clone()),
            )?);
            let result = futures_util::future::poll_fn(|cx| {
                if cancelled() {
                    return std::task::Poll::Ready(None);
                }
                std::future::Future::poll(pending.as_mut(), cx).map(Some)
            })
            .await;
            let Some(result) = result else {
                return Ok(None);
            };
            let image = match result {
                Ok(image) => image,
                Err(error) if matches!(error.as_ref(), Error::Cancelled) => return Ok(None),
                Err(error) => return Err(Error::Gpu(error.to_string())),
            };
            image.image.admit(admission.clone());
            scope.release_optional(&image.identity);
            images.push(image);
        }
        Ok(Some(ActiveInput { images }))
    }
}
