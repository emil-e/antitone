use super::*;
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
    )
    .map(|_| ())
}

impl Editor {
    #[cfg(test)]
    pub(super) fn neutral_source_signature(
        &self,
        input: &InputSettings,
    ) -> Result<crate::gpu::interpretation::InputTransform, Error> {
        self.document
            .visit
            .canvas
            .ready
            .as_ref()
            .ok_or(Error::InvalidParameter("positive source unavailable"))?;
        let recipe = self.positive_recipe(
            *input,
            self.document.project_snapshot().roll,
            self.photo_settings(),
        )?;
        crate::gpu::interpretation::InputTransform::from_recipe(&recipe.neutral)
    }

    pub(super) fn validate_document_candidate(&self, document: &Document) -> Result<(), Error> {
        let mut candidate = document.clone();
        candidate.capture_initialization(&self.document.initialization);
        candidate.validate_with_device(self.document.visit.working_view.device())
    }

    pub(super) fn validate_source_candidate(&self, source: &Source) -> Result<(), Error> {
        let recipe = self.capture_recipe(
            source,
            self.input(),
            self.document.project_snapshot().roll,
            source.grade_snapshot()?,
        )?;
        if recipe.neutral.input.ready() {
            recipe.validate(
                &self
                    .document
                    .visit
                    .working_view
                    .device()
                    .map(wgpu::Device::limits)
                    .unwrap_or_default(),
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
        let grade = self.adjustment_layer_snapshot(source, photo)?;
        self.capture_recipe(source, input.into(), roll, grade)
    }

    fn capture_recipe(
        &self,
        source: &Source,
        input: InputSettings,
        roll: RollSettings,
        grade: AdjustmentLayerSnapshot,
    ) -> Result<crate::image_recipe::ImageRecipe, Error> {
        let project = self.document.project_snapshot();
        source
            .repairs
            .validate(source.development.active_area.into())?;
        let repairs = self
            .dust_recipe_repairs(source)
            .unwrap_or_else(|| source.repairs.repairs().to_vec())
            .into_iter()
            .filter(|repair| repair.enabled)
            .collect();
        Ok(crate::image_recipe::ImageRecipe {
            neutral: crate::image_recipe::NeutralRecipe {
                source: source.id,
                source_development: self.document.development_for(source, input),
                input,
                reference: self.document.exposure_value(source),
                correction: project
                    .active_flat_field()
                    .map(|correction| correction.map.clone()),
            },
            roll,
            grade,
            composition: Composition::with_crop(
                source.development.active_area.into(),
                project.orientation,
                source.orientation,
                source.crop,
            )?,
            grain: project.output.grain,
            halation: project.output.halation,
            luts: project.output.luts(),
            repairs,
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
        let project = self.document.project_snapshot();
        self.positive_recipe_for_source(input, roll, project.source(id)?, photo)
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
        for source in &self.document.state.project.sources {
            let recipe = self.positive_recipe_for_source(input, roll, source, photo)?;
            if input.ready() {
                recipe.validate(
                    &self
                        .document
                        .visit
                        .working_view
                        .device()
                        .map(wgpu::Device::limits)
                        .unwrap_or_default(),
                )?;
            }
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
