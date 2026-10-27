use super::*;
use crate::processing::InputSettings;
use crate::{
    geometry::SourceRegion,
    gpu::{GpuPipeline, ProcessingInput},
};
use std::cell::RefCell;

pub(super) type NeutralSourceSignature = crate::gpu::InputSignature;

#[derive(PartialEq)]
struct Key {
    input: InputSettings,
    roll: RollSettings,
    matrix: glam::DMat3,
    area: SourceRegion,
}

struct Entry {
    id: SourceId,
    key: Key,
    grade: Arc<GpuPipeline>,
}

#[derive(Default)]
pub(super) struct Grades {
    device: Option<wgpu::Device>,
    entries: RefCell<Vec<Entry>>,
}

impl Grades {
    pub(super) fn new(device: wgpu::Device) -> Self {
        Self {
            device: Some(device),
            ..Default::default()
        }
    }

    pub(super) fn clear(&self) {
        self.entries.borrow_mut().clear();
    }

    pub(super) fn retain(&self, sources: &[SourceId]) {
        self.entries
            .borrow_mut()
            .retain(|entry| sources.contains(&entry.id));
    }

    fn prepare(
        &self,
        id: SourceId,
        key: Key,
        snapshot: AdjustmentLayerSnapshot,
    ) -> Result<ProcessingInput, Error> {
        if let Some(entry) =
            self.entries.borrow().iter().find(|entry| {
                entry.id == id && entry.key == key && entry.grade.snapshot() == &snapshot
            })
        {
            return Ok(entry.grade.clone().into());
        }
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| Error::Gpu("no editor GPU device".into()))?;
        let grade = Arc::new(GpuPipeline::new(
            device,
            key.input,
            key.roll,
            snapshot.clone(),
            key.matrix,
            key.area,
        )?);
        let input = grade.clone().into();
        let mut entries = self.entries.borrow_mut();
        entries.retain(|entry| entry.id != id);
        entries.push(Entry { id, key, grade });
        Ok(input)
    }
}

#[cfg(test)]
pub(super) fn validate_source(
    device: Option<&wgpu::Device>,
    input: impl Into<InputSettings>,
    roll: RollSettings,
    source: &Source,
) -> Result<(), Error> {
    validate_source_with_matrix(
        device,
        input.into(),
        roll,
        source,
        glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1),
    )
}

pub(super) fn validate_source_with_matrix(
    device: Option<&wgpu::Device>,
    input: InputSettings,
    roll: RollSettings,
    source: &Source,
    matrix: glam::DMat3,
) -> Result<(), Error> {
    let snapshot = source.grade_snapshot()?;
    if !input.ready() {
        return Ok(());
    }
    if let Some(device) = device {
        GpuPipeline::validate(
            device,
            input,
            roll,
            &snapshot,
            matrix,
            source.development.active_area.into(),
        )
    } else {
        GpuPipeline::validate_limits(
            &wgpu::Limits::default(),
            input,
            roll,
            &snapshot,
            matrix,
            source.development.active_area.into(),
        )
    }
}

impl Editor {
    pub(super) fn neutral_source_signature(
        &self,
        input: &InputSettings,
    ) -> Result<NeutralSourceSignature, Error> {
        let ready = self
            .document
            .visit
            .ready
            .as_ref()
            .ok_or(Error::InvalidParameter("positive source unavailable"))?;
        let mut scan = Scan::from_source(ready.developed.clone());
        scan.set_flat_field(
            self.correction
                .as_ref()
                .map(|correction| correction.settings.map.clone()),
        )?;
        crate::scan::ImageSource::new(&scan, *input)?
            .gpu_program()?
            .signature()
    }

    pub(super) fn validate_repaired_source(
        &self,
        input: &InputSettings,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        bound: DVec3,
    ) -> Result<(), Error> {
        if bound == DVec3::ZERO {
            return Ok(());
        }
        let (Some(device), Some(ready)) = (self.working.device(), &self.document.visit.ready)
        else {
            return Ok(());
        };
        let input = match input.calibration() {
            Some(base) => input.with_calibration(self.processing_calibration(base)?),
            None => *input,
        };
        crate::gpu::GpuPipeline::validate_repaired(
            device,
            input,
            roll,
            snapshot,
            ready.developed.positive_matrix(&input)?,
            ready.area,
            bound,
        )
    }

    pub(super) fn validate_document_candidate(&self, document: &Document) -> Result<(), Error> {
        document.validate_with_device(self.grades.device.as_ref())
    }

    pub(super) fn validate_source_candidate(&self, source: &Source) -> Result<(), Error> {
        let mut input = self.input();
        if let Some(base) = input.calibration() {
            input = input.with_calibration(source.development.processing_calibration(
                base,
                self.correction.as_ref().map(|c| c.settings.map.as_ref()),
            )?);
        }
        validate_source_with_matrix(
            self.grades.device.as_ref(),
            input,
            self.roll,
            source,
            self.document.positive_matrix(source, input)?,
        )?;
        if self.current_source == Some(source.id) {
            self.dust_validate_snapshot(input, self.roll, &source.grade_snapshot()?)?;
        }
        Ok(())
    }

    pub(super) fn positive_processing_for_source(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        source: &Source,
        photo: AdjustmentLayerSettings,
    ) -> Result<ProcessingInput, Error> {
        let input = input.into();
        let mut candidate = source.clone();
        self.overlay_adjustment_layer_settings(&mut candidate, photo);
        let mut recipe = self
            .document
            .project
            .image_recipe_for_source(&candidate, &input, roll)?;
        recipe.film_emulation = self.film_emulation;
        recipe.neutral.correction = self
            .correction
            .as_ref()
            .map(|correction| correction.settings.map.clone());
        let snapshot = recipe.grade;
        let look = recipe.look;
        let input = recipe.neutral.processing_input()?;
        let matrix = self.document.positive_matrix(source, input)?;
        self.grades
            .prepare(
                source.id,
                Key {
                    input,
                    roll,
                    matrix,
                    area: source.development.active_area.into(),
                },
                snapshot,
            )
            .map(|input| {
                input
                    .with_look(look)
                    .with_film_emulation(recipe.film_emulation)
                    .with_frame_size(recipe.composition.crop().size)
                    .with_film_seed(source.id.0)
            })
    }

    pub(super) fn positive_processing(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: AdjustmentLayerSettings,
    ) -> Result<ProcessingInput, Error> {
        match self.current_source {
            Some(id) => self.positive_processing_for_source(
                input,
                roll,
                self.document.project.source(id)?,
                photo,
            ),
            None => self.positive_parameters(input, roll).map(|parameters| {
                ProcessingInput::from(parameters)
                    .with_look(self.document.project.output.look.clone())
                    .with_film_emulation(self.film_emulation)
            }),
        }
    }

    /// Roll/input candidates must support every source, without replacing accepted resources.
    pub(super) fn validate_processing(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: AdjustmentLayerSettings,
    ) -> Result<(), Error> {
        let input = input.into();
        photo.validate()?;
        self.positive_parameters(input, roll)?;
        self.dust_validate_processing(input, roll, photo)?;
        for source in &self.document.project.sources {
            let mut candidate = source.clone();
            self.overlay_adjustment_layer_settings(&mut candidate, photo);
            let source_input = match input.calibration() {
                Some(base) => input.with_calibration(source.development.processing_calibration(
                    base,
                    self.correction.as_ref().map(|c| c.settings.map.as_ref()),
                )?),
                None => input,
            };
            validate_source_with_matrix(
                self.grades.device.as_ref(),
                source_input,
                roll,
                &candidate,
                self.document.positive_matrix(&candidate, source_input)?,
            )?;
        }
        Ok(())
    }
}

pub(super) fn image_paint(
    image: Arc<GpuImage>,
    input: Option<ProcessingInput>,
    mapping: crate::geometry::TextureMapping,
    context: &egui::Context,
) -> Result<ImagePaint, Error> {
    if input
        .as_ref()
        .is_some_and(|input| input.pipeline().is_some() || input.look().is_some())
    {
        ImagePaint::with_processing(image, input, mapping)
            .map(|paint| paint.with_context(context.clone()))
    } else {
        Ok(
            ImagePaint::new(image, input.map(|input| input.parameters()), mapping)
                .with_context(context.clone()),
        )
    }
}

#[cfg(test)]
pub(super) mod tests;
