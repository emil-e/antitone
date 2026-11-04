use super::*;
use crate::gpu::parameters::ProcessingParameters;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::processing::InputSettings;

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
    crate::gpu::photograph::admission::prepare(
        &device.map(|device| device.limits()).unwrap_or_default(),
        input,
        roll,
        &snapshot,
        matrix,
        source.development.active_area.into(),
        DVec3::ZERO,
    )
    .map(|_| ())
}

impl Editor {
    pub(super) fn positive_parameters(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
    ) -> Result<ProcessingParameters, Error> {
        let basis = self
            .document
            .project
            .camera_basis()
            .ok_or(Error::UnusableSample("no native-camera basis identity"))?;
        let matrix = glam::DMat3::from_cols_array_2d(&basis.camera_to_ap1);
        self.positive_parameters_for_matrix(input, roll, matrix)
    }

    pub(super) fn positive_parameters_for_matrix(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        camera_to_ap1: glam::DMat3,
    ) -> Result<ProcessingParameters, Error> {
        let input = input.into();
        let input = match input.calibration() {
            Some(base) => input.with_calibration(self.processing_calibration(base)?),
            None => input,
        };
        ProcessingParameters::interpret(input, roll, camera_to_ap1)
    }

    pub(super) fn processing_calibration(
        &self,
        calibration: Calibration,
    ) -> Result<Calibration, Error> {
        let source_floor = self
            .document
            .visit
            .current_source
            .map(|id| {
                self.document
                    .state
                    .project
                    .source(id)
                    .map(|s| s.development.input_floor)
            })
            .transpose()?
            .unwrap_or(crate::processing::CAMERA_INPUT_FLOOR);
        let residual_gain = self
            .document
            .correction
            .as_ref()
            .map_or(1., |c| c.maximum_gain);
        calibration.for_source(source_floor, residual_gain)
    }

    pub(super) fn neutral_source_signature(
        &self,
        input: &InputSettings,
    ) -> Result<crate::gpu::interpretation::InputTransform, Error> {
        let ready = self
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .ok_or(Error::InvalidParameter("positive source unavailable"))?;
        let mut scan = Scan::with_reference(ready.developed.clone(), ready.reference.clone());
        scan.set_flat_field(
            self.document
                .correction
                .as_ref()
                .map(|correction| correction.settings.map.clone()),
        )?;
        crate::gpu::interpretation::InputTransform::for_scan(&scan, *input)
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
        let (Some(device), Some(ready)) = (
            self.document.visit.working_view.device(),
            &self.document.visit.canvas.ready,
        ) else {
            return Ok(());
        };
        let input = match input.calibration() {
            Some(base) => input.with_calibration(self.processing_calibration(base)?),
            None => *input,
        };
        crate::gpu::photograph::admission::prepare(
            &device.limits(),
            input,
            roll,
            snapshot,
            input.positive_matrix(
                ready.developed.camera_to_ap1(),
                ready.reference.as_ref().map(|reference| reference.ev()),
            )?,
            ready.area,
            bound,
        )
        .map(|_| ())
    }

    pub(super) fn validate_document_candidate(&self, document: &Document) -> Result<(), Error> {
        let mut candidate = document.clone();
        candidate.capture_exposure(&self.document.exposures);
        candidate.validate_with_device(self.document.visit.working_view.device())
    }

    pub(super) fn validate_source_candidate(&self, source: &Source) -> Result<(), Error> {
        let mut input = self.input();
        if let Some(base) = input.calibration() {
            input = input.with_calibration(
                source.development.processing_calibration(
                    base,
                    self.document
                        .correction
                        .as_ref()
                        .map(|c| c.settings.map.as_ref()),
                )?,
            );
        }
        validate_source_with_matrix(
            self.document.visit.working_view.device(),
            input,
            self.document.roll,
            source,
            self.document.positive_matrix(source, input)?,
        )?;
        if self.document.visit.current_source == Some(source.id) {
            self.dust_validate_snapshot(input, self.document.roll, &source.grade_snapshot()?)?;
        }
        Ok(())
    }

    pub(super) fn positive_recipe_for_source(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        source: &Source,
        photo: AdjustmentLayerSettings,
    ) -> Result<crate::image_recipe::ImageRecipe, Error> {
        let input = input.into();
        let mut candidate = source.clone();
        self.overlay_adjustment_layer_settings(&mut candidate, photo);
        let mut recipe = self
            .document
            .project
            .image_recipe_for_source(&candidate, &input, roll)?;
        recipe.neutral.reference = self.document.exposure_value(source);
        recipe.grain = self.document.grain;
        recipe.halation = self.document.halation;
        recipe.neutral.correction = self
            .document
            .correction
            .as_ref()
            .map(|correction| correction.settings.map.clone());
        Ok(recipe)
    }

    #[cfg(test)]
    pub(super) fn positive_processing_for_source(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        source: &Source,
        photo: AdjustmentLayerSettings,
    ) -> Result<AuthoredPhotograph, Error> {
        let recipe = self.positive_recipe_for_source(input, roll, source, photo)?;
        let limits = self
            .document
            .visit
            .working_view
            .device()
            .map(|device| device.limits())
            .unwrap_or_default();
        AuthoredPhotograph::from_recipe(&limits, &recipe)
    }

    pub(super) fn positive_recipe(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: AdjustmentLayerSettings,
    ) -> Result<crate::image_recipe::ImageRecipe, Error> {
        let id = self
            .document
            .visit
            .current_source
            .ok_or(Error::UnusableSample("no photographic source selected"))?;
        self.positive_recipe_for_source(input, roll, self.document.state.project.source(id)?, photo)
    }

    #[cfg(test)]
    pub(super) fn positive_processing(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: AdjustmentLayerSettings,
    ) -> Result<AuthoredPhotograph, Error> {
        match self.document.visit.current_source {
            Some(id) => self.positive_processing_for_source(
                input,
                roll,
                self.document.state.project.source(id)?,
                photo,
            ),
            None => Err(Error::UnusableSample("no photographic source selected")),
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
        for source in &self.document.state.project.sources {
            let mut candidate = source.clone();
            self.overlay_adjustment_layer_settings(&mut candidate, photo);
            let source_input = match input.calibration() {
                Some(base) => input.with_calibration(
                    source.development.processing_calibration(
                        base,
                        self.document
                            .correction
                            .as_ref()
                            .map(|c| c.settings.map.as_ref()),
                    )?,
                ),
                None => input,
            };
            validate_source_with_matrix(
                self.document.visit.working_view.device(),
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
    capture: Option<crate::gpu::photograph::request::PhotographRequest>,
    field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
    mapping: crate::geometry::TextureMapping,
    context: &egui::Context,
) -> Result<ImagePaint, Error> {
    use crate::gpu::photograph::request::PhotographRequest;
    let (root, luts) = match capture {
        Some(PhotographRequest::Recipe(recipe)) => (
            Some(image.recipe_calculation(&recipe, field.clone())?),
            recipe.luts.clone(),
        ),
        Some(PhotographRequest::Inspection(view)) => (
            Some(image.inspection(view.parameters()?, field)?),
            Default::default(),
        ),
        None => (None, Default::default()),
        #[cfg(test)]
        Some(PhotographRequest::Fixture(input)) => {
            return ImagePaint::with_processing(image, Some(input.with_flat_field(field)), mapping)
                .map(|paint| paint.with_context(context.clone()));
        }
    };
    ImagePaint::with_calculation(image, root, luts, mapping)
        .map(|paint| paint.with_context(context.clone()))
}

#[cfg(test)]
mod immutable_source_tests;
#[cfg(test)]
pub(super) mod tests;
