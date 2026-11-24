//! Photographic stage order, reusable prefixes and quality variants.
//!
//! `Assembly::processed` is the shared post-grade sequence. `RecipeGraph` places reconstruction
//! and selects repairs/emulation for original samples, physical views, camera proxies and measurements.
//! Inspection branches before inversion; request routing and operation mathematics have separate owners.

use super::assembly::SourceImages;
use super::operations::{Native, Working};
use super::*;
#[cfg(feature = "project")]
use crate::gpu::interpretation::InputTransform;
#[cfg(feature = "project")]
use crate::image_recipe::{ImageRecipe, quality::Quality};

#[cfg(all(test, feature = "editor"))]
mod sequence_tests;
#[cfg(all(test, feature = "project", feature = "render"))]
mod tests;

/// Reconstruction onto the presentation grid in a physical-view graph.
#[derive(Clone, Copy)]
pub(crate) struct ViewOutput {
    domain: Domain,
    offset: [u32; 2],
    scale: [f64; 2],
}

impl ViewOutput {
    #[cfg(feature = "editor")]
    pub(super) fn new(
        presentation: crate::frame_raster::FrameRaster,
        processing: crate::frame_raster::FrameRaster,
        coverage: u32,
    ) -> Self {
        Self {
            domain: view_domain(presentation, coverage),
            offset: [0; 2],
            scale: std::array::from_fn(|i| {
                f64::from(processing.output[i]) / f64::from(presentation.output[i])
            }),
        }
    }

    pub(crate) fn compose(self, image: Value) -> Result<Value, Error> {
        if image.domain().same_grid(self.domain) {
            window(image, self.domain.finite()?)
        } else {
            reconstruct_cells(image, self.domain, self.offset.map(f64::from), self.scale)
        }
    }
}

pub(super) struct Finished {
    neutral: Value,
    roll: Value,
    graded: Value,
    pub(super) processed: Value,
}

impl Finished {
    pub(super) fn with_source(self, source: SourceImages) -> PhotographGraph {
        PhotographGraph {
            codes: source.codes,
            camera: source.camera,
            camera_positive: source.camera_positive,
            unrepaired: source.unrepaired,
            neutral: self.neutral,
            roll: self.roll,
            graded: self.graded,
            processed: self.processed,
        }
    }
}

/// A captured source branch remains usable before inversion or grading is ready.
/// Asking for positive endpoints performs the corresponding numerical admission.
#[cfg(feature = "project")]
pub(crate) struct RecipeGraph<'a> {
    recipe: &'a ImageRecipe,
    source: CameraGraph,
    quality: Quality,
}

#[cfg(feature = "project")]
impl<'a> RecipeGraph<'a> {
    pub fn originals(
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        recipe: &'a ImageRecipe,
    ) -> Result<Self, Error> {
        PhotographGraph::require_recipe_source(&source, recipe)?;
        Ok(Self {
            recipe,
            source: CameraGraph::new(
                source,
                CameraInput {
                    matrix: glam::DMat3::from_cols_array_2d(
                        &recipe.neutral.source_development.resolved()?.camera_to_ap1,
                    ),
                    field,
                    continuous: true,
                    mapping: Mapping::Originals,
                },
            ),
            quality: Quality::Originals,
        })
    }

    #[cfg(all(feature = "ui", feature = "render"))]
    pub fn sampled_camera(
        codes: Value,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        recipe: &'a ImageRecipe,
    ) -> Result<Self, Error> {
        let expected = glam::DMat3::from_cols_array_2d(
            &recipe.neutral.source_development.resolved()?.camera_to_ap1,
        );
        if source.camera_matrix() != expected {
            return Err(Error::Gpu(
                "recipe differs from resident camera basis".into(),
            ));
        }
        Ok(Self {
            recipe,
            source: CameraGraph::new(
                codes,
                CameraInput {
                    matrix: expected,
                    field,
                    continuous: source.continuous,
                    mapping: Mapping::Resident(source.geometry()),
                },
            ),
            quality: Quality::CameraProxy,
        })
    }

    fn require_field(field: Option<&Arc<GpuFlatField>>, recipe: &ImageRecipe) -> Result<(), Error> {
        if recipe.neutral.correction.as_ref() != field.map(|field| field.field()) {
            return Err(Error::Gpu(
                "recipe and GPU flat-field resources differ".into(),
            ));
        }
        Ok(())
    }

    /// Original-grid measurements omit repairs and display emulation.
    #[cfg(feature = "editor")]
    pub(crate) fn measurement(mut self) -> Self {
        self.quality = Quality::Measurement;
        self
    }

    pub(crate) fn codes(&self) -> Value {
        self.source.codes.clone()
    }

    pub(crate) fn camera(&self) -> Result<Value, Error> {
        Self::require_field(self.source.input.field.as_ref(), self.recipe)?;
        self.source.camera()
    }

    pub(crate) fn camera_positive(&self) -> Result<Value, Error> {
        self.interpretation()?.interpret(self.camera()?)
    }

    pub(crate) fn unrepaired(&self) -> Result<Value, Error> {
        self.interpretation()?.working(self.camera_positive()?)
    }

    pub(crate) fn neutral(&self) -> Result<Value, Error> {
        self.repair(self.unrepaired()?)
    }

    pub(crate) fn roll(&self) -> Result<Value, Error> {
        Grading::roll_image(self.neutral()?, self.recipe.roll)
    }

    pub(crate) fn graded(&self, limits: &wgpu::Limits) -> Result<Value, Error> {
        let assembly = self.assembly(limits, 0)?;
        assembly
            .grading
            .as_ref()
            .expect("photographic grading")
            .layers(self.roll()?)
    }

    pub(crate) fn processed(&self, limits: &wgpu::Limits) -> Result<Value, Error> {
        self.assembly(limits, 0)?
            .processed(self.graded(limits)?, None)
    }

    #[cfg(feature = "render")]
    pub(crate) fn display(
        &self,
        limits: &wgpu::Limits,
        looks: &crate::gpu::lut3d::Looks,
        drt: &crate::gpu::drt::Target,
    ) -> Result<Value, Error> {
        display_output(self.processed(limits)?, &self.recipe.luts, looks, drt)
    }

    #[cfg(feature = "editor")]
    pub(super) fn view_definition(
        limits: &wgpu::Limits,
        recipe: &ImageRecipe,
    ) -> Result<Assembly, Error> {
        let area = recipe.neutral.source_development.active_area().into();
        let grading = Grading::new(limits, recipe.roll, recipe.grade.clone(), area)?;
        let mut definition = Assembly::filtered(grading);
        definition.resolve_source_scale(recipe)?;
        Ok(definition)
    }

    fn interpretation(&self) -> Result<Assembly, Error> {
        let program = InputTransform::from_recipe(&self.recipe.neutral)?;
        let mut camera = self.source.input.clone();
        camera.matrix = program.matrix;
        Ok(Assembly::captured(Captured::new(
            camera,
            Interpretation::Program(program),
        )?))
    }

    pub(crate) fn interpreted(&self) -> Result<SourceImages, Error> {
        self.interpretation()?
            .interpret_camera(self.codes(), self.camera()?)
    }

    fn assembly(&self, limits: &wgpu::Limits, coverage: u32) -> Result<Assembly, Error> {
        let program = InputTransform::from_recipe(&self.recipe.neutral)?;
        let area = self.recipe.neutral.source_development.active_area().into();
        let mut camera = self.source.input.clone();
        camera.matrix = program.matrix;
        let mut grading = Grading::new(limits, self.recipe.roll, self.recipe.grade.clone(), area)?
            .for_grid(coverage);
        if let Mapping::Resident(geometry) = camera.mapping {
            grading.require_geometry(geometry)?;
            if let Some(geometry) = geometry.filter(|geometry| geometry.is_pair())
                && geometry.regions().any(|region| {
                    region.x < area.x
                        || region.y < area.y
                        || region.x >= area.x + area.width
                        || region.y >= area.y + area.height
                })
            {
                return Err(Error::Gpu(
                    "composed input and source geometry regions differ".into(),
                ));
            }
            grading = grading.with_camera_coordinates(geometry);
        }
        let mut assembly =
            Assembly::photographic(limits, camera, program, grading, Default::default())?;
        #[cfg(feature = "editor")]
        if self.quality == Quality::Measurement {
            return Ok(assembly);
        }
        assembly.resolve_source_scale(self.recipe)?;
        Ok(assembly)
    }

    fn repair(&self, image: Value) -> Result<Value, Error> {
        if self.quality.includes_repairs() {
            crate::gpu::repairs::definition::compose(image, &self.recipe.repairs)
        } else {
            Ok(image)
        }
    }

    /// Original samples: correction → interpretation → AP1 → repairs → Roll → layers
    /// → MTF → halation → grain. Already-sampled camera inputs use the same stages,
    /// but their approximation has already averaged camera samples and omits repairs.
    #[cfg(test)]
    pub fn photograph(&self, limits: &wgpu::Limits) -> Result<PhotographGraph, Error> {
        let assembly = self.assembly(limits, 0)?;
        let source = assembly.interpret_camera(self.codes(), self.camera()?)?;
        let neutral = self.repair(source.unrepaired.clone())?;
        Ok(assembly.finish(neutral, None)?.with_source(source))
    }

    #[cfg(feature = "editor")]
    pub fn physical_view(
        &self,
        limits: &wgpu::Limits,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(PhotographGraph, crate::acquisition::Plan), Error> {
        let assembly = self.assembly(limits, 8)?;
        let layout = assembly
            .view_sampling(frame, crate::acquisition::Sampling::NeutralFootprints)?
            .with_native_grid(native_grid, |input| assembly.sampling_image(input))?;
        let plan = layout.plan;
        let source = assembly.interpret_camera(self.codes(), self.camera()?)?;
        let repaired = self.repair(source.unrepaired.clone())?;
        // Repair before filtering. The filtered positive is reusable across grading edits.
        let neutral = crate::gpu::evaluation::reconstruct_tiled(
            repaired,
            Domain {
                bounds: None,
                grid: Grid {
                    coverage: 8,
                    ..Grid::from_frame(plan.output())
                },
            },
        )?
        .for_reuse();
        let output = ViewOutput::new(layout.presentation, plan.output(), 8);
        Ok((
            assembly.finish(neutral, Some(output))?.with_source(source),
            plan,
        ))
    }

    #[cfg(all(feature = "ui", feature = "render"))]
    pub fn sampled_raster(
        source: &ProcessingSource,
        recipe: &ImageRecipe,
        layout: [u32; 2],
    ) -> Result<crate::gpu::raster::Raster, Error> {
        if source.geometry().is_none()
            && (recipe
                .grain
                .parameters(recipe.composition.crop().size)?
                .is_some()
                || recipe
                    .halation
                    .parameters(recipe.composition.crop().size)?
                    .is_some())
        {
            return Err(Error::Gpu(
                "photographic processing requires declared raster geometry".into(),
            ));
        }
        PhotographGraph::resident_raster(source, layout)
    }
}

impl Assembly {
    pub fn photograph(self, source: Value) -> Result<PhotographGraph, Error> {
        let images = self.source_images(source)?;
        let neutral = self.repair(images.unrepaired.clone())?;
        Ok(self.finish(neutral, None)?.with_source(images))
    }

    pub(super) fn finish(
        &self,
        neutral: Value,
        output: Option<ViewOutput>,
    ) -> Result<Finished, Error> {
        let (roll, graded) = self.grade(neutral.clone())?;
        let processed = self.processed(graded.clone(), output)?;
        Ok(Finished {
            neutral,
            roll,
            graded,
            processed,
        })
    }

    #[cfg(all(test, feature = "editor"))]
    pub fn preview(
        mut self,
        source: Value,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(PhotographGraph, crate::acquisition::Plan), Error> {
        let layout = self
            .view_sampling(frame, crate::acquisition::Sampling::NeutralFootprints)?
            .with_native_grid(native_grid, |input| self.sampling_image(input))?;
        let plan = layout.plan;
        let native = self.source_images(source)?;
        let repaired = self.repair(native.unrepaired.clone())?;
        let neutral = reconstruct_tiled(
            repaired,
            Domain {
                bounds: None,
                grid: Grid {
                    coverage: 8,
                    ..Grid::from_frame(plan.output())
                },
            },
        )?
        .for_reuse();
        self.grading = self.grading.map(|grading| grading.for_grid(8));
        let output = ViewOutput::new(layout.presentation, plan.output(), 8);
        Ok((
            self.finish(neutral, Some(output))?.with_source(native),
            plan,
        ))
    }

    #[cfg(all(test, feature = "editor"))]
    pub fn filtered_image(
        self,
        source: Value,
        output: Domain,
        offset: [u32; 2],
        scale: [f64; 2],
    ) -> Result<Value, Error> {
        let (_, graded) = self.grade(source)?;
        self.processed(
            graded,
            Some(ViewOutput {
                domain: output,
                offset,
                scale,
            }),
        )
    }

    pub fn camera(&self, source: Value) -> Value {
        Value::from_operation(
            source.domain(),
            Some(source),
            Camera {
                captured: self.capture.as_ref().expect("camera metadata").clone(),
            },
        )
    }

    pub fn interpret(&self, camera: Value) -> Result<Value, Error> {
        Ok(Value::from_operation(
            camera.domain(),
            Some(camera),
            Native::new(self.interpretation)?,
        ))
    }

    pub(super) fn working(&self, native: Value) -> Result<Value, Error> {
        let capture = self.capture.as_ref().ok_or(Error::InvalidParameter(
            "working conversion lacks captured camera metadata",
        ))?;
        Ok(Value::from_operation(
            native.domain(),
            Some(native),
            Working::new(capture.interpretation, capture.matrix)?,
        ))
    }

    pub fn source_images(&self, codes: Value) -> Result<SourceImages, Error> {
        let camera = self.camera(codes.clone());
        self.interpret_camera(codes, camera)
    }

    pub fn interpret_camera(&self, codes: Value, camera: Value) -> Result<SourceImages, Error> {
        let camera_positive = self.interpret(camera.clone())?;
        let unrepaired = self.working(camera_positive.clone())?;
        Ok(SourceImages {
            codes,
            camera,
            camera_positive,
            unrepaired,
        })
    }

    pub fn unrepaired(&self, codes: Value) -> Result<Value, Error> {
        Ok(self.source_images(codes)?.unrepaired)
    }

    pub fn repair(&self, image: Value) -> Result<Value, Error> {
        let needed = !self.repairs.is_empty();
        #[cfg(test)]
        let needed = needed || !self.replacements.is_empty();
        if !needed {
            return Ok(image);
        }
        let domain = image.domain();
        // Resident original-cell arguments have local indices; masks name absolute source cells.
        let image = if domain.grid != Grid::axis_aligned([0.; 2], [1.; 2]) {
            let Some(Mapping::Resident(Some(geometry))) =
                self.capture.as_ref().map(|capture| capture.mapping)
            else {
                return Err(Error::InvalidParameter(
                    "healing requires original neutral cells",
                ));
            };
            let region = geometry.region();
            if geometry.is_pair()
                || geometry.size() != [region.width, region.height]
                || domain.grid.axes != [[1., 0.], [0., 1.]]
                || domain.grid.origin != [f64::from(region.x), f64::from(region.y)]
            {
                return Err(Error::InvalidParameter(
                    "healing requires original neutral cells",
                ));
            }
            crate::gpu::evaluation::reconstruct(
                image,
                Domain {
                    bounds: Some(region.try_into()?),
                    grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                },
            )?
        } else {
            image
        };
        let image = if self.repairs.is_empty() {
            image
        } else {
            crate::gpu::repairs::definition::compose(image, &self.repairs)?
        };
        #[cfg(test)]
        let image = if self.replacements.is_empty() {
            image
        } else {
            crate::gpu::repairs::definition::compose_images(image, self.replacements.clone())?
        };
        if image.domain() == domain {
            Ok(image)
        } else {
            crate::gpu::evaluation::reconstruct(image, domain)
        }
    }

    pub fn neutral(&self, codes: Value) -> Result<Value, Error> {
        self.repair(self.unrepaired(codes)?)
    }

    pub fn grade(&self, neutral: Value) -> Result<(Value, Value), Error> {
        let Some(grading) = &self.grading else {
            return Ok((neutral.clone(), neutral));
        };
        let roll = grading.roll(neutral)?;
        let graded = grading.layers(roll.clone())?;
        Ok((roll, graded))
    }

    pub(crate) fn processed(
        &self,
        mut image: Value,
        output: Option<ViewOutput>,
    ) -> Result<Value, Error> {
        #[cfg(test)]
        if let Some(author) = &self.author {
            return author(image, output);
        }
        if let Some(film) = self.grain {
            image = crate::gpu::gaussian::blur(image, film.image_mtf_sigma)?;
        }
        if let Some(halation) = self.halation {
            image = halation.compose(image)?;
        }
        if let Some(output) = output {
            image = output.compose(image)?;
        }
        if let Some(film) = self.grain.filter(|film| film.amplitude > 0.) {
            image = film.grain().compose(image, self.seed);
        }
        #[cfg(test)]
        let image = super::authoring::suffix(image)?;
        Ok(image)
    }

    pub(crate) fn sampling_image(&self, input: Value) -> Result<Value, Error> {
        let graded = if self.grading.is_some() {
            self.grade(input)?.1
        } else {
            Grading::roll_image(input, Default::default())?
        };
        self.processed(graded, None)
    }

    #[cfg(any(test, feature = "editor"))]
    pub(crate) fn view_sampling(
        &self,
        frame: crate::frame_raster::FrameRaster,
        sampling: crate::acquisition::Sampling,
    ) -> Result<super::sampling::ViewSampling, Error> {
        let pitch = self
            .grain
            .map(|film| film.image_mtf_sigma)
            .into_iter()
            .chain(
                self.halation
                    .filter(|halo| halo.strength > 0.)
                    .map(|halo| halo.sigma),
            )
            .reduce(f64::min)
            .map(|sigma| sigma / super::sampling::GAUSSIAN_VIEW_CELLS);
        #[cfg(test)]
        let pitch = self.preferred_pitch.or(pitch);
        super::sampling::ViewSampling::new(frame, pitch, sampling, |input| {
            self.sampling_image(input)
        })
    }
}

#[cfg(feature = "project")]
impl PhotographGraph {
    #[cfg(feature = "editor")]
    pub(crate) fn preview(
        limits: &wgpu::Limits,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        recipe: &crate::image_recipe::ImageRecipe,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(Self, crate::acquisition::Plan), Error> {
        let program = crate::gpu::interpretation::InputTransform::from_recipe(&recipe.neutral)?;
        if !program.matches_native(scan)?
            || scan.flat_field() != recipe.neutral.correction.as_ref()
            || recipe.neutral.correction.as_ref() != field.as_ref().map(|field| field.field())
        {
            return Err(Error::Gpu(
                "physical-view source authority differs from originals".into(),
            ));
        }
        let source = crate::gpu::source::camera_source(scan)?;
        if frame.composition.area() != source.domain().finite()?.source()? {
            return Err(Error::InvalidParameter(
                "frame belongs to a different source grid",
            ));
        }
        RecipeGraph::originals(source, field, recipe)?.physical_view(limits, frame, native_grid)
    }

    #[cfg(test)]
    pub(crate) fn from_recipe(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        recipe: &ImageRecipe,
    ) -> Result<Self, Error> {
        RecipeGraph::originals(source, field, recipe)?.photograph(limits)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn filtered_recipe(
        limits: &wgpu::Limits,
        spec: &crate::gpu::view::request::SourceRequest,
        neutral: Value,
        recipe: &ImageRecipe,
    ) -> Result<(Value, crate::acquisition::Plan), Error> {
        let program = InputTransform::from_recipe(&recipe.neutral)?;
        let area = recipe.neutral.source_development.active_area().into();
        let grading = Grading::new(limits, recipe.roll, recipe.grade.clone(), area)?;
        grading.require_extent(spec.area)?;
        if Self::input_transform(&neutral)?.is_some_and(|captured| captured != program) {
            return Err(Error::Gpu("physical-view source is stale".into()));
        }
        admission::prepare(
            limits,
            program.input,
            recipe.roll,
            &recipe.grade,
            program.matrix,
            area,
        )?;
        let mut assembly = Assembly::filtered(grading.for_grid(neutral.domain().grid.coverage));
        assembly.resolve_source_scale(recipe)?;
        let plan = spec.processing_plan(&assembly)?;
        let output = ViewOutput::new(spec.frame(), plan.output(), neutral.domain().grid.coverage);
        let finished = assembly.finish(neutral, Some(output))?;
        Ok((finished.processed, plan))
    }

    fn require_recipe_source(source: &Value, recipe: &ImageRecipe) -> Result<(), Error> {
        if source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
            || source.domain().finite()?.source()?
                != recipe.neutral.source_development.active_area().into()
        {
            return Err(Error::InvalidParameter(
                "recipe requires its original camera cells",
            ));
        }
        Ok(())
    }
}

impl CameraGraph {
    pub(super) fn inspection(
        &self,
        view: crate::gpu::interpretation::InspectionView,
    ) -> Result<Value, Error> {
        Ok(Assembly::captured(Captured::new(
            self.input.clone(),
            Interpretation::Packed(view.parameters()?),
        )?)
        .interpret_camera(self.codes.clone(), self.camera()?)?
        .unrepaired)
    }
}

impl PhotographGraph {
    /// Capture original-cell operations; these arguments are also used by analytic consumers.
    /// Author a complete original-grid working-positive calculation from operation arguments.
    pub fn original_image(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        program: crate::gpu::interpretation::InputTransform,
        grading: Grading,
        repairs: &[crate::dust_repair::Repair],
    ) -> Result<Value, Error> {
        Ok(Self::original(limits, source, field, program, grading, repairs.to_vec())?.processed)
    }

    pub(crate) fn original(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        program: crate::gpu::interpretation::InputTransform,
        grading: Grading,
        repairs: Vec<crate::dust_repair::Repair>,
    ) -> Result<Self, Error> {
        if source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2]) {
            return Err(Error::InvalidParameter(
                "photographic conversion requires original camera cells",
            ));
        }
        grading.require_extent(source.domain().finite()?.source()?)?;
        Assembly::photographic(
            limits,
            CameraInput {
                matrix: program.matrix,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
            program,
            grading,
            repairs,
        )?
        .photograph(source)
    }

    pub(crate) fn inspection_original(
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        matrix: glam::DMat3,
        view: crate::gpu::interpretation::InspectionView,
    ) -> Result<Value, Error> {
        if source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2]) {
            return Err(Error::InvalidParameter(
                "inspection requires original camera cells",
            ));
        }
        CameraGraph::new(
            source,
            CameraInput {
                matrix,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
        )
        .inspection(view)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn inspection_view(
        neutral: Value,
        parameters: ProcessingParameters,
        presentation: crate::frame_raster::FrameRaster,
    ) -> Result<Value, Error> {
        let output = view_domain(presentation, neutral.domain().grid.coverage);
        window(
            crate::gpu::interpretation::inspect_filtered(neutral.for_reuse(), parameters)?,
            output.finite()?,
        )
    }
}

#[cfg(feature = "render")]
pub fn display_output(
    image: Value,
    luts: &crate::looks::LutChain,
    looks: &crate::gpu::lut3d::Looks,
    drt: &crate::gpu::drt::Target,
) -> Result<Value, Error> {
    drt.apply(looks.apply(image, luts)?)
}

#[cfg(feature = "render")]
pub(crate) fn prewarm_output(
    context: &Context,
    drt: &crate::gpu::drt::Targets,
) -> Result<(), Error> {
    use crate::rendering::OutputTarget;

    let input = borrowed_input(Domain {
        bounds: Some(CellRegion::new([0, 0], [1, 1])),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
    });
    let looked = crate::gpu::lut3d::prewarm_value(input.clone())?;
    context.prewarm_image(looked.clone())?;
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let display = drt.get(target);
        context.prewarm_texture(display.apply(input.clone())?)?;
        context.prewarm_texture(display.apply(looked.clone())?)?;
    }
    Ok(())
}
