//! Capture resident camera calculations and record their result into an image's target storage.

use super::*;
use crate::gpu::{
    evaluation::{ImageNode, Meaning},
    grading::Grading,
    interpretation::{InputTransform, InspectionView},
    photograph::{PhotographGraph, operations::CameraInput},
    raster::Raster,
};
use crate::{looks::LutChain, pixel_pipeline::PixelPipeline};

impl ImageProcessing {
    fn camera(&self, raster: Raster) -> Result<Arc<ImageNode>, Error> {
        let mut camera = self.camera.lock().unwrap();
        let domain = crate::gpu::evaluation::Domain {
            bounds: Some(crate::gpu::evaluation::CellRegion::new(
                [0, 0],
                raster.size(),
            )),
            grid: crate::gpu::evaluation::Grid::from_raster(raster),
            meaning: Meaning::CameraCodes,
        };
        if let Some(camera) = camera.as_ref().filter(|camera| camera.domain() == domain) {
            return Ok(camera.clone());
        }
        let captured = PhotographGraph::resident_codes(&self.source, raster)?;
        *camera = Some(captured.clone());
        Ok(captured)
    }
}

impl GpuImage {
    /// Author a calculation over this image's declared camera cells.
    pub fn photograph(
        &self,
        program: InputTransform,
        grading: Grading,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
        stages: &PixelPipeline,
    ) -> Result<Arc<ImageNode>, Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        let raster = PhotographGraph::resident_raster(
            &processing.source,
            stages,
            [self.presentation.size[0], self.presentation.size[1]],
        )?;
        Ok(PhotographGraph::resident_photograph(
            &self.device_limits,
            processing.camera(raster)?,
            CameraInput::resident(&processing.source, program, field),
            program,
            grading,
            Default::default(),
            stages,
        )?
        .processed)
    }

    pub fn inspection(
        &self,
        parameters: ProcessingParameters,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
    ) -> Result<Arc<ImageNode>, Error> {
        let view = InspectionView::from_parameters(parameters)?;
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        let raster = PhotographGraph::resident_raster(
            &processing.source,
            &Default::default(),
            [self.presentation.size[0], self.presentation.size[1]],
        )?;
        PhotographGraph::resident_inspection(
            processing.camera(raster)?,
            &processing.source,
            field,
            view,
        )
    }

    #[cfg(feature = "editor")]
    pub(crate) fn recipe_calculation(
        &self,
        recipe: &crate::image_recipe::ImageRecipe,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
    ) -> Result<Arc<ImageNode>, Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        let expected = DMat3::from_cols_array_2d(&recipe.neutral.source_development.camera_to_ap1);
        if processing.source.camera_matrix() != expected {
            return Err(Error::Gpu(
                "recipe differs from resident camera basis".into(),
            ));
        }
        if field.as_ref().map(|field| field.field()) != recipe.neutral.correction.as_ref() {
            return Err(Error::Gpu(
                "recipe and GPU flat-field resources differ".into(),
            ));
        }
        let program = InputTransform::from_recipe(&recipe.neutral)?;
        let stages = recipe.pixel_pipeline()?;
        let raster = PhotographGraph::resident_raster(
            &processing.source,
            &stages,
            [self.presentation.size[0], self.presentation.size[1]],
        )?;
        Ok(PhotographGraph::resident_photograph(
            &self.device_limits,
            processing.camera(raster)?,
            CameraInput::resident(&processing.source, program, field),
            program,
            Grading::new(
                &self.device_limits,
                recipe.roll,
                recipe.grade.clone(),
                recipe.neutral.source_development.active_area.into(),
            )?,
            Default::default(),
            &stages,
        )?
        .processed)
    }

    #[cfg(test)]
    pub(crate) fn legacy_calculation(
        &self,
        input: &AuthoredPhotograph,
    ) -> Result<Arc<ImageNode>, Error> {
        self.require_processing(input)?;
        let Some(program) = input.source_transform() else {
            return self.inspection(input.parameters(), input.field.clone());
        };
        let processing = self.processing.as_ref().expect("admitted camera source");
        let stages = input.pixel_pipeline();
        let raster = PhotographGraph::resident_raster(
            &processing.source,
            &stages,
            [self.presentation.size[0], self.presentation.size[1]],
        )?;
        Ok(PhotographGraph::resident_photograph(
            &self.device_limits,
            processing.camera(raster)?,
            CameraInput::resident(&processing.source, *program, input.field.clone()),
            *program,
            input.grade().expect("admitted photograph").clone(),
            input.repairs.clone(),
            &stages,
        )?
        .processed)
    }

    pub(crate) fn prepared_definition(&self) -> Option<Arc<ImageNode>> {
        self.processing
            .as_ref()?
            .last_calculation
            .lock()
            .unwrap()
            .clone()
    }

    pub fn require_calculation(&self, root: &Arc<ImageNode>, luts: &LutChain) -> Result<(), Error> {
        if self.processing.is_none() {
            return Err(Error::Gpu("rendered image has no source buffer".into()));
        }
        if root.domain().finite()?
            != crate::gpu::evaluation::CellRegion::new(
                [0, 0],
                [self.presentation.size[0], self.presentation.size[1]],
            )
            || !matches!(
                root.domain().meaning,
                Meaning::Neutral | Meaning::Graded | Meaning::Inspection
            )
        {
            return Err(Error::InvalidParameter(
                "calculation differs from working image storage",
            ));
        }
        for asset in luts.assets() {
            crate::gpu::lut3d::Lut::require_limits(&self.device_limits, asset.lut())?;
        }
        Ok(())
    }

    /// Viewer and scopes share calculations over this image's resident input.
    pub fn prepare_calculation(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        root: &Arc<ImageNode>,
        luts: &LutChain,
    ) -> Result<bool, Error> {
        self.require_calculation(root, luts)?;
        let processing = self.processing.as_ref().expect("admitted working image");
        let mut previous = processing.last_calculation.lock().unwrap();
        let same_pixels = previous
            .as_ref()
            .is_some_and(|old| old.same_definition(root));
        let same_look = processing
            .aces_group
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|binding| binding.look_id() == luts.id());
        if same_pixels && same_look {
            return Ok(false);
        }
        self.record_calculation(device, encoder, root.clone(), luts)?;
        *previous = Some(root.clone());
        Ok(true)
    }

    pub fn encode_calculation(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        root: &Arc<ImageNode>,
        luts: &LutChain,
    ) -> Result<(), Error> {
        self.require_calculation(root, luts)?;
        let processing = self.processing.as_ref().expect("admitted working image");
        let mut previous = processing.last_calculation.lock().unwrap();
        self.record_calculation(device, encoder, root.clone(), luts)?;
        *previous = None;
        Ok(())
    }

    fn record_calculation(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        root: Arc<ImageNode>,
        luts: &LutChain,
    ) -> Result<(), Error> {
        let processing = self.processing.as_ref().expect("admitted working image");
        let raster = root.domain().grid.raster(root.domain().finite()?);
        let prepared = crate::gpu::pixel_pipeline::Prepared::from_composition(
            device,
            raster,
            root.clone(),
            processing.renderer.evaluation.clone(),
        )?;
        prepared.encode_result(device, encoder, &processing.working)?;
        let prepared_binding = processing.renderer.render_binding(
            device,
            crate::gpu::evaluation::SuppliedImage::from_buffer(
                root.clone(),
                root.domain().finite()?,
                processing.working.clone().into(),
            )?,
            processing.target,
            luts,
            &self.linear,
        )?;
        let check = prepared_binding.encode(device, encoder)?;
        *processing.aces_group.lock().unwrap() = Some(prepared_binding);
        self.look_projections.lock().unwrap().clear();
        *self.look_check.lock().unwrap() = check;
        Ok(())
    }
}
