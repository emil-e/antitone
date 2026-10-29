use super::adjustment_layers::{stage_bounds, supported};
use super::{GpuGrade, GpuInputProgram, ProcessingParameters, ProcessingSource};
use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::SourceRegion,
    processing::{InputSettings, RollSettings},
};
use glam::{DMat3, DVec3};

/// Captured composition of source interpretation and photographic grading.
pub struct GpuPipeline {
    pub(super) parameters: ProcessingParameters,
    input: std::sync::Arc<GpuInputProgram>,
    pub(super) grade: std::sync::Arc<GpuGrade>,
    area: SourceRegion,
}

impl GpuPipeline {
    /// Check the complete candidate and adapter limits without allocating GPU resources.
    pub fn validate(
        device: &wgpu::Device,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
    ) -> Result<(), Error> {
        Self::validate_limits(&device.limits(), input, roll, snapshot, camera_to_ap1, area)
    }

    pub fn validate_limits(
        limits: &wgpu::Limits,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
    ) -> Result<(), Error> {
        Self::prepare(limits, &input.into(), roll, snapshot, camera_to_ap1, area).map(|_| ())
    }

    pub fn new(
        device: &wgpu::Device,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        snapshot: AdjustmentLayerSnapshot,
        matrix: DMat3,
        area: SourceRegion,
    ) -> Result<Self, Error> {
        let input = GpuInputProgram::new(input, matrix)?;
        let parameters = Self::prepare(
            &device.limits(),
            &input.input,
            roll,
            &snapshot,
            input.matrix,
            area,
        )?;
        let grade = std::sync::Arc::new(GpuGrade::new(device, roll, snapshot, area)?);
        Ok(Self {
            parameters,
            input: std::sync::Arc::new(input),
            grade,
            area,
        })
    }

    pub fn compose(
        device: &wgpu::Device,
        input: GpuInputProgram,
        grade: std::sync::Arc<GpuGrade>,
        area: SourceRegion,
    ) -> Result<Self, Error> {
        grade.require_extent(area)?;
        let parameters = Self::prepare(
            &device.limits(),
            &input.input,
            grade.roll,
            &grade.snapshot,
            input.matrix,
            area,
        )?;
        Ok(Self {
            parameters,
            input: std::sync::Arc::new(input),
            grade,
            area,
        })
    }

    #[cfg(any(feature = "editor", all(feature = "raw", feature = "render")))]
    pub(crate) fn validate_repaired(
        device: &wgpu::Device,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
        neutral_bound: DVec3,
    ) -> Result<(), Error> {
        Self::prepare_with_neutral_bound(
            &device.limits(),
            &input.into(),
            roll,
            snapshot,
            camera_to_ap1,
            area,
            neutral_bound,
        )
        .map(|_| ())
    }

    fn prepare(
        limits: &wgpu::Limits,
        input: &InputSettings,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
    ) -> Result<ProcessingParameters, Error> {
        Self::prepare_with_neutral_bound(
            limits,
            input,
            roll,
            snapshot,
            camera_to_ap1,
            area,
            DVec3::ZERO,
        )
    }

    fn prepare_with_neutral_bound(
        limits: &wgpu::Limits,
        input: &InputSettings,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
        neutral_bound: DVec3,
    ) -> Result<ProcessingParameters, Error> {
        if !neutral_bound.is_finite() || neutral_bound.min_element() < 0. {
            return Err(Error::Gpu("invalid neutral repair bound".into()));
        }
        let area = area.validate()?;
        let (parameters, mut bound) = ProcessingParameters::interpret_with_neutral_bound(
            *input,
            roll,
            camera_to_ap1,
            neutral_bound,
        )?;
        GpuGrade::validate(limits, roll, snapshot, area)?;
        for adjustment in snapshot.layers().iter().copied().filter(|a| a.is_active()) {
            let (peak, output) = stage_bounds(adjustment, bound)?;
            supported(peak)?;
            bound = output;
        }
        Ok(parameters)
    }

    #[cfg(any(feature = "editor", all(feature = "raw", feature = "render")))]
    pub(crate) fn validate_repairs(
        &self,
        device: &wgpu::Device,
        bound: DVec3,
    ) -> Result<(), Error> {
        Self::validate_repaired(
            device,
            self.input.input,
            self.grade.roll,
            &self.grade.snapshot,
            self.input.matrix,
            self.area,
            bound,
        )
    }

    pub fn input_program(&self) -> &GpuInputProgram {
        &self.input
    }

    pub fn snapshot(&self) -> &AdjustmentLayerSnapshot {
        self.grade.snapshot()
    }

    #[cfg(feature = "editor")]
    pub(crate) fn byte_len_for(snapshot: &AdjustmentLayerSnapshot) -> u64 {
        let active = snapshot
            .layers()
            .iter()
            .filter(|layer| layer.is_active())
            .count()
            .max(1);
        (active * std::mem::size_of::<super::adjustment_layers::AdjustmentLayerParameters>()
            + std::mem::size_of::<super::adjustment_layers::AdjustmentLayerContext>())
            as u64
    }

    pub fn byte_len(&self) -> u64 {
        self.grade.byte_len()
    }

    pub(super) fn same_content(&self, other: &Self) -> bool {
        self.input == other.input
            && self.grade.roll == other.grade.roll
            && self.snapshot() == other.snapshot()
            && self.area == other.area
    }

    pub(super) fn require_source(&self, source: &ProcessingSource) -> Result<(), Error> {
        self.input.require_source(source)?;
        self.grade.require_geometry(source.geometry)?;
        if let Some(geometry) = source.geometry.filter(|g| g.is_pair()) {
            for region in geometry.regions() {
                if region.x < self.area.x
                    || region.y < self.area.y
                    || region.x >= self.area.x + self.area.width
                    || region.y >= self.area.y + self.area.height
                {
                    return Err(Error::Gpu(
                        "composed input and source geometry regions differ".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}
