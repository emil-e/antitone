//! Capture resident camera calculations and record their result into an image's target storage.

use super::*;
use crate::looks::LutChain;
use crate::photographic::graph::resident;
use crate::photographic::stages::{
    grading::Grading,
    interpretation::{InputTransform, InspectionView},
};
use crate::{acquisition::raster::Raster, evaluation::Value};

impl ImageProcessing {
    fn camera(&self, raster: Raster) -> Result<Value, Error> {
        let mut camera = self.camera.lock().unwrap();
        let domain = crate::evaluation::Domain {
            bounds: Some(crate::evaluation::CellRegion::new([0, 0], raster.size())),
            grid: crate::evaluation::Grid::from_raster(raster),
        };
        if let Some(camera) = camera.as_ref().filter(|camera| camera.domain() == domain) {
            return Ok(camera.clone());
        }
        let captured = resident::codes(&self.source, raster)?;
        *camera = Some(captured.clone());
        Ok(captured)
    }
}

impl GpuImage {
    /// Select target-linear output for an explicitly requested photographic prefix.
    pub fn output(&self, prefix: Value, luts: &LutChain) -> Result<Value, Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        crate::photographic::graph::display_output(
            prefix,
            luts,
            &processing.looks,
            &processing.transform,
        )
    }

    /// Author a calculation over this image's declared camera cells.
    pub fn photograph(
        &self,
        program: InputTransform,
        grading: Grading,
        field: Option<Arc<crate::photographic::stages::flat_field::GpuFlatField>>,
    ) -> Result<Value, Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        let raster = resident::raster(&processing.source, self.size())?;
        resident::photograph(
            &self.device_limits,
            processing.camera(raster)?,
            &processing.source,
            field,
            program,
            grading,
            &[],
        )
    }

    pub fn inspection(
        &self,
        parameters: ProcessingParameters,
        field: Option<Arc<crate::photographic::stages::flat_field::GpuFlatField>>,
    ) -> Result<Value, Error> {
        let view = InspectionView::from_parameters(parameters)?;
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        let raster = resident::raster(&processing.source, self.size())?;
        resident::inspection(processing.camera(raster)?, &processing.source, field, view)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn capture_calculation(
        &self,
        capture: &crate::photographic::graph::request::PhotographRequest,
        field: Option<Arc<crate::photographic::stages::flat_field::GpuFlatField>>,
    ) -> Result<Value, Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        capture.resident_output(
            &self.device_limits,
            &processing.source,
            self.size(),
            field,
            &processing.looks,
            &processing.transform,
        )
    }

    #[cfg(test)]
    pub(crate) fn processed_prefix(
        &self,
        recipe: &crate::image_recipe::ImageRecipe,
        field: Option<Arc<crate::photographic::stages::flat_field::GpuFlatField>>,
    ) -> Result<Value, Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        crate::photographic::graph::request::PhotographRequest::Recipe(Arc::new(recipe.clone()))
            .resident_processed(&self.device_limits, &processing.source, self.size(), field)
    }

    #[cfg(test)]
    pub(crate) fn legacy_calculation(&self, input: &AuthoredPhotograph) -> Result<Value, Error> {
        self.require_processing(input)?;
        let processing = self.processing.as_ref().expect("admitted camera source");
        let image = if let Some(program) = input.source_transform() {
            let stages = input.definition();
            let raster = resident::raster(&processing.source, self.size())?;
            stages.compose(resident::photograph(
                &self.device_limits,
                processing.camera(raster)?,
                &processing.source,
                input.field.clone(),
                *program,
                input.grade().expect("admitted photograph").clone(),
                &input.repairs,
            )?)?
        } else {
            self.inspection(input.parameters(), input.field.clone())?
        };
        crate::photographic::graph::display_output(
            image,
            input.luts(),
            &processing.looks,
            &processing.transform,
        )
    }

    pub(crate) fn prepared_definition(&self) -> Option<Value> {
        self.processing
            .as_ref()?
            .last_calculation
            .lock()
            .unwrap()
            .clone()
    }

    pub fn require_calculation(&self, root: &Value, luts: &LutChain) -> Result<(), Error> {
        if self.processing.is_none() {
            return Err(Error::Gpu("rendered image has no source buffer".into()));
        }
        if root.domain().finite()? != crate::evaluation::CellRegion::new([0, 0], self.size()) {
            return Err(Error::InvalidParameter(
                "calculation differs from working image storage",
            ));
        }
        for asset in luts.assets() {
            crate::photographic::stages::look::Lut::require_limits(
                &self.device_limits,
                asset.lut(),
            )?;
        }
        Ok(())
    }

    /// Viewer and scopes share calculations over this image's resident input.
    pub fn prepare_calculation(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        root: &Value,
        luts: &LutChain,
    ) -> Result<bool, Error> {
        self.require_calculation(root, luts)?;
        let processing = self.processing.as_ref().expect("admitted working image");
        let mut previous = processing.last_calculation.lock().unwrap();
        let same_pixels = previous
            .as_ref()
            .is_some_and(|old| old.same_definition(root));
        if same_pixels {
            return Ok(false);
        }
        self.record_calculation(device, encoder, root.clone())?;
        *previous = Some(root.clone());
        Ok(true)
    }

    pub fn encode_calculation(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        root: &Value,
        luts: &LutChain,
    ) -> Result<(), Error> {
        self.require_calculation(root, luts)?;
        let processing = self.processing.as_ref().expect("admitted working image");
        let mut previous = processing.last_calculation.lock().unwrap();
        self.record_calculation(device, encoder, root.clone())?;
        *previous = None;
        Ok(())
    }

    fn record_calculation(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        root: Value,
    ) -> Result<(), Error> {
        let processing = self.processing.as_ref().expect("admitted working image");
        let raster = root.domain().grid.raster(root.domain().finite()?);
        let prepared = crate::evaluation::recorded_image::Prepared::from_composition(
            device,
            raster,
            root.clone(),
            processing.evaluation.clone(),
        )?;
        prepared.encode_result(device, encoder, &processing.working)?;
        let prepared_binding = crate::evaluation::recorded_image::Texture::new(
            &processing.evaluation,
            device,
            crate::evaluation::SuppliedImage::from_buffer(
                root.clone(),
                root.domain().finite()?,
                processing.working.clone().into(),
            )?,
            root,
            &self.linear,
        )?;
        prepared_binding.encode(device, encoder)?;
        *processing.recording.lock().unwrap() = Some(prepared_binding);
        Ok(())
    }
}
