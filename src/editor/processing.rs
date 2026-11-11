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
        let input = input.into();
        let matrix = if let Some(id) = self.document.visit.current_source {
            let development = self
                .document
                .development_for(self.document.project.source(id)?, input);
            glam::DMat3::from_cols_array_2d(&development.resolved()?.camera_to_ap1)
        } else {
            glam::DMat3::from_cols_array_2d(
                &self
                    .document
                    .scan_basis()
                    .ok_or(Error::UnusableSample("no native-camera basis identity"))?
                    .camera
                    .camera_to_ap1,
            )
        };
        self.positive_parameters_for_matrix(input, roll, matrix)
    }

    pub(super) fn positive_parameters_for_matrix(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        camera_to_ap1: glam::DMat3,
    ) -> Result<ProcessingParameters, Error> {
        let input = input.into();
        ProcessingParameters::interpret(input, roll, camera_to_ap1)
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
                .project
                .active_flat_field()
                .map(|correction| correction.map.clone()),
        );
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
        let input = *input;
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
        candidate.capture_initialization(&self.document.initialization);
        candidate.validate_with_device(self.document.visit.working_view.device())
    }

    pub(super) fn validate_source_candidate(&self, source: &Source) -> Result<(), Error> {
        let input = self.input();
        validate_source_with_matrix(
            self.document.visit.working_view.device(),
            input,
            self.document.project_snapshot().roll,
            source,
            self.document.positive_matrix(source, input)?,
        )?;
        if self.document.visit.current_source == Some(source.id) {
            self.dust_validate_snapshot(
                input,
                self.document.project_snapshot().roll,
                &source.grade_snapshot()?,
            )?;
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
        self.document.project.capture_image(&candidate, |recipe| {
            recipe.neutral.input = input;
            recipe.neutral.source_development = self.document.development_for(&candidate, input);
            recipe.roll = roll;
            recipe.neutral.reference = self.document.exposure_value(source);
            recipe.grain = self.document.project_snapshot().output.grain;
            recipe.halation = self.document.project_snapshot().output.halation;
            if let Some(repairs) = self.dust_recipe_repairs(&candidate) {
                recipe.repairs = repairs
                    .into_iter()
                    .filter(|repair| repair.enabled)
                    .collect();
            }
        })
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
            let source_input = input;
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
        Some(capture @ (PhotographRequest::Recipe(_) | PhotographRequest::BeforeLooks(_))) => (
            Some(image.recipe_calculation(
                capture.recipe().expect("photographic request"),
                field.clone(),
            )?),
            capture.luts().clone(),
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
