use super::*;
use crate::processing::InputSettings;
use crate::{
    geometry::SourceRegion,
    gpu::{GpuPhotoGrade, ProcessingInput},
};
use std::cell::RefCell;

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
    grade: Arc<GpuPhotoGrade>,
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

    fn remove(&self, id: SourceId) {
        self.entries.borrow_mut().retain(|entry| entry.id != id);
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
        snapshot: PhotoGradeSnapshot,
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
        let grade = Arc::new(GpuPhotoGrade::new(
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

fn snapshot(source: &Source, photo: PhotoSettings) -> Result<PhotoGradeSnapshot, Error> {
    PhotoGradeSnapshot::new(photo, source.grade_snapshot()?.locals().to_vec())
}

pub(super) fn validate_source(
    device: Option<&wgpu::Device>,
    input: &InputSettings,
    roll: RollSettings,
    source: &Source,
    photo: PhotoSettings,
) -> Result<(), Error> {
    let snapshot = snapshot(source, photo)?;
    let matrix = glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1);
    if snapshot.locals().is_empty() {
        if input.is_negative() && input.calibration().is_none() {
            // A fresh Negative project is valid before Dmin/Dmax are measured. Rendering
            // reports the missing calibration; import/save/validation can still proceed.
            return Ok(());
        }
        ProcessingParameters::interpret(input, roll, photo, matrix).map(|_| ())
    } else {
        if input.is_negative() && input.calibration().is_none() {
            return Err(Error::InvalidParameter(
                "local adjustments require a calibrated processing basis",
            ));
        }
        let device = device
            .ok_or_else(|| Error::Gpu("local adjustments require an editor GPU device".into()))?;
        GpuPhotoGrade::validate(
            device,
            input,
            roll,
            &snapshot,
            matrix,
            source.development.active_area.into(),
        )
    }
}

impl Editor {
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
        validate_source(
            self.grades.device.as_ref(),
            &input,
            self.roll,
            source,
            source.photo,
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
        photo: PhotoSettings,
    ) -> Result<ProcessingInput, Error> {
        let input = input.into();
        let mut recipe = self
            .document
            .project
            .image_recipe_for_source(source, &input, roll, photo)?;
        recipe.neutral.correction = self
            .correction
            .as_ref()
            .map(|correction| correction.settings.map.clone());
        let snapshot = recipe.grade;
        let look = recipe.look;
        let matrix =
            glam::DMat3::from_cols_array_2d(&recipe.neutral.source_development.camera_to_ap1);
        let input = recipe.neutral.processing_input()?;
        if snapshot.locals().is_empty() && self.grades.device.is_none() {
            // CPU-only fixtures have no GPU grade allocation; production always uses the
            // complete grade path so local adjustments are never an optional suffix.
            let parameters = ProcessingParameters::interpret(input, roll, photo, matrix)?;
            self.grades.remove(source.id);
            return Ok(ProcessingInput::from(parameters).with_look(look));
        }
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
            .map(|input| input.with_look(look))
    }

    pub(super) fn positive_processing(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: PhotoSettings,
    ) -> Result<ProcessingInput, Error> {
        match self.current_source {
            Some(id) => self.positive_processing_for_source(
                input,
                roll,
                self.document.project.source(id)?,
                photo,
            ),
            None => self
                .positive_parameters(input, roll, photo)
                .map(|parameters| {
                    ProcessingInput::from(parameters)
                        .with_look(self.document.project.output.look.clone())
                }),
        }
    }

    /// Roll/input candidates must support every source, without replacing accepted resources.
    pub(super) fn validate_processing(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: PhotoSettings,
    ) -> Result<(), Error> {
        let input = input.into();
        self.positive_parameters(input, roll, photo)?;
        self.dust_validate_processing(input, roll, photo)?;
        for source in &self.document.project.sources {
            let photo = if self.current_source == Some(source.id) {
                photo
            } else {
                source.photo
            };
            let source_input = match input.calibration() {
                Some(base) => input.with_calibration(source.development.processing_calibration(
                    base,
                    self.correction.as_ref().map(|c| c.settings.map.as_ref()),
                )?),
                None => input,
            };
            validate_source(
                self.grades.device.as_ref(),
                &source_input,
                roll,
                source,
                photo,
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
        .is_some_and(|input| input.grade().is_some() || input.look().is_some())
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
#[path = "editor_processing_tests.rs"]
pub(super) mod tests;
