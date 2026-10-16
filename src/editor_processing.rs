use super::*;
use crate::{
    geometry::SourceRegion,
    gpu::{GpuPhotoGrade, ProcessingInput},
};
use std::cell::RefCell;

#[derive(PartialEq)]
struct Key {
    calibration: Calibration,
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
            key.calibration,
            key.roll,
            snapshot,
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
    calibration: Calibration,
    roll: RollSettings,
    source: &Source,
    photo: PhotoSettings,
) -> Result<(), Error> {
    let snapshot = snapshot(source, photo)?;
    let matrix = glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1);
    if snapshot.locals().is_empty() {
        ProcessingParameters::positive(calibration, roll, photo, matrix).map(|_| ())
    } else {
        let device = device
            .ok_or_else(|| Error::Gpu("local adjustments require an editor GPU device".into()))?;
        GpuPhotoGrade::validate(
            device,
            calibration,
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
        let calibration = self.calibration().ok_or(Error::InvalidParameter(
            "local adjustments require calibration",
        ))?;
        validate_source(
            self.grades.device.as_ref(),
            self.processing_calibration(calibration)?,
            self.roll,
            source,
            source.photo,
        )?;
        if self.current_source == Some(source.id) {
            self.dust_validate_snapshot(calibration, self.roll, &source.grade_snapshot()?)?;
        }
        Ok(())
    }

    pub(super) fn positive_processing_for_source(
        &self,
        calibration: Calibration,
        roll: RollSettings,
        source: &Source,
        photo: PhotoSettings,
    ) -> Result<ProcessingInput, Error> {
        let snapshot = snapshot(source, photo)?;
        let matrix = glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1);
        let calibration = self.processing_calibration(calibration)?;
        if snapshot.locals().is_empty() {
            let parameters = ProcessingParameters::positive(calibration, roll, photo, matrix)?;
            self.grades.remove(source.id);
            return Ok(parameters.into());
        }
        self.grades.prepare(
            source.id,
            Key {
                calibration,
                roll,
                matrix,
                area: source.development.active_area.into(),
            },
            snapshot,
        )
    }
    pub(super) fn positive_processing(
        &self,
        calibration: Calibration,
        roll: RollSettings,
        photo: PhotoSettings,
    ) -> Result<ProcessingInput, Error> {
        match self.current_source {
            Some(id) => self.positive_processing_for_source(
                calibration,
                roll,
                self.document.project.source(id)?,
                photo,
            ),
            None => self
                .positive_parameters(calibration, roll, photo)
                .map(Into::into),
        }
    }

    /// Roll/calibration candidates must support every source, without replacing accepted resources.
    pub(super) fn validate_processing(
        &self,
        calibration: Calibration,
        roll: RollSettings,
        photo: PhotoSettings,
    ) -> Result<(), Error> {
        self.positive_parameters(calibration, roll, photo)?;
        self.dust_validate_processing(calibration, roll, photo)?;
        let calibration = self.processing_calibration(calibration)?;
        for source in &self.document.project.sources {
            let photo = if self.current_source == Some(source.id) {
                photo
            } else {
                source.photo
            };
            validate_source(
                self.grades.device.as_ref(),
                calibration,
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
) -> Result<ImagePaint, Error> {
    if input.as_ref().is_some_and(|input| input.grade().is_some()) {
        ImagePaint::with_processing(image, input, mapping)
    } else {
        Ok(ImagePaint::new(
            image,
            input.map(|input| input.parameters()),
            mapping,
        ))
    }
}

#[cfg(test)]
#[path = "editor_processing_tests.rs"]
pub(super) mod tests;
