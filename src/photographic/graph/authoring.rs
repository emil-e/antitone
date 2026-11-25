//! Fixture graph authoring uses the ordinary operation functions.
#[cfg(feature = "editor")]
use super::composition::reconstruct_neutral;
use super::composition::{
    PresentationMapping, film_effects, interpret_camera, preferred_view_pitch, repair_neutral,
};
use super::*;
use crate::gpu::test_inputs::AuthoredPhotograph;
#[cfg(feature = "project")]
use crate::image_recipe::ImageRecipe;

/// Named calculation endpoints used by photographic fixtures.
pub(crate) struct PhotographTaps {
    pub codes: Value,
    pub unrepaired: Value,
    pub neutral: Value,
    pub roll: Value,
    pub graded: Value,
    pub(crate) processed: Value,
}

#[derive(Clone, Default)]
pub(crate) struct Assembly {
    pub grading: Option<Grading>,
    pub repairs: Vec<crate::dust_repair::Repair>,
    pub grain: Option<crate::photographic::stages::grain::GrainResponse>,
    pub halation: Option<crate::photographic::stages::halation::HalationParameters>,
    pub seed: u64,
    pub author: Option<Arc<Author>>,
    pub preferred_pitch: Option<f64>,
    pub replacements: Vec<crate::gpu::repairs::fixtures::Replacement>,
}

impl Assembly {
    #[cfg(feature = "project")]
    pub(crate) fn resolve_source_scale(
        &mut self,
        recipe: &crate::image_recipe::ImageRecipe,
    ) -> Result<(), Error> {
        (self.grain, self.halation) = super::composition::film_parameters(recipe)?;
        self.seed = recipe.neutral.source.0;
        Ok(())
    }
}

pub(super) struct SourceImages {
    pub codes: Value,
    pub unrepaired: Value,
}

impl SourceImages {
    pub(super) fn from_codes(
        codes: Value,
        field: Option<Arc<GpuFlatField>>,
        mapping: Mapping,
        matrix: glam::DMat3,
        continuous: bool,
        interpretation: Interpretation,
    ) -> Result<Self, Error> {
        validate_camera_range(matrix, field.as_deref(), continuous)?;
        let camera = camera_image(codes.clone(), field, mapping);
        Self::from_camera(codes, camera, interpretation, matrix)
    }

    pub(super) fn from_camera(
        codes: Value,
        camera: Value,
        interpretation: Interpretation,
        matrix: glam::DMat3,
    ) -> Result<Self, Error> {
        let unrepaired = interpret_camera(camera, interpretation, matrix)?;
        Ok(Self { codes, unrepaired })
    }

    pub(super) fn finish(
        self,
        neutral: Value,
        grading: Option<&Grading>,
        process: impl FnOnce(Value) -> Result<Value, Error>,
    ) -> Result<PhotographTaps, Error> {
        let (roll, graded) = if let Some(grading) = grading {
            let roll = grading.roll(neutral.clone())?;
            let graded = grading.layers(roll.clone())?;
            (roll, graded)
        } else {
            (neutral.clone(), neutral.clone())
        };
        let processed = process(graded.clone())?;
        Ok(PhotographTaps {
            codes: self.codes,
            unrepaired: self.unrepaired,
            neutral,
            roll,
            graded,
            processed,
        })
    }
}

impl Assembly {
    pub(super) fn photograph(
        self,
        images: SourceImages,
        mapping: Mapping,
    ) -> Result<PhotographTaps, Error> {
        let neutral = self.repair(images.unrepaired.clone(), mapping)?;
        images.finish(neutral, self.grading.as_ref(), |graded| {
            self.processed(graded, None)
        })
    }

    #[cfg(feature = "editor")]
    pub(super) fn preview(
        mut self,
        images: SourceImages,
        mapping: Mapping,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(PhotographTaps, crate::acquisition::Plan), Error> {
        let layout = self
            .view_sampling(frame, crate::acquisition::Sampling::NeutralFootprints)?
            .with_native_grid(native_grid, |input| self.sampling_image(input))?;
        let plan = layout.plan;
        let repaired = self.repair(images.unrepaired.clone(), mapping)?;
        let neutral = reconstruct_neutral(repaired, plan.output())?;
        self.grading = self.grading.map(|grading| grading.for_grid(8));
        let output = PresentationMapping::new(layout.presentation, plan.output(), 8);
        let graph = images.finish(neutral, self.grading.as_ref(), |graded| {
            self.processed(graded, Some(output))
        })?;
        Ok((graph, plan))
    }

    fn repair(&self, image: Value, mapping: Mapping) -> Result<Value, Error> {
        repair_neutral(image, mapping, &self.repairs, &self.replacements)
    }

    pub(crate) fn processed(
        &self,
        image: Value,
        output: Option<PresentationMapping>,
    ) -> Result<Value, Error> {
        if let Some(author) = &self.author {
            return author(image, output);
        }
        film_effects(image, self.grain, self.halation, self.seed, output)
    }

    pub(crate) fn sampling_image(&self, input: Value) -> Result<Value, Error> {
        let graded = match &self.grading {
            Some(grading) => grading.compose(input)?,
            None => Grading::roll_image(input, Default::default())?,
        };
        self.processed(graded, None)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn processing_plan(
        &self,
        source: &super::view_source::SourceRequest,
    ) -> Result<crate::acquisition::Plan, Error> {
        super::sampling::processing_plan(source, self.view_pitch(), |input| {
            self.sampling_image(input)
        })
    }

    pub(crate) fn view_sampling(
        &self,
        frame: crate::frame_raster::FrameRaster,
        sampling: crate::acquisition::Sampling,
    ) -> Result<super::sampling::ViewSampling, Error> {
        super::sampling::ViewSampling::new(frame, self.view_pitch(), sampling, |input| {
            self.sampling_image(input)
        })
    }

    fn view_pitch(&self) -> Option<f64> {
        self.preferred_pitch
            .or_else(|| preferred_view_pitch(self.grain, self.halation))
    }
}

#[cfg(all(feature = "raw", feature = "render", feature = "editor"))]
pub(crate) fn original_chain(
    limits: &wgpu::Limits,
    scan: &crate::scan::Scan,
    field: Option<Arc<GpuFlatField>>,
    input: &crate::gpu::test_inputs::AuthoredPhotograph,
    stages: &Assembly,
    looks: &crate::photographic::stages::look::Looks,
    target: &crate::photographic::stages::display_rendering::Target,
) -> Result<Value, Error> {
    if scan.flat_field() != field.as_ref().map(|f| f.field()) {
        return Err(Error::Gpu("CPU and GPU flat-field resources differ".into()));
    }
    let matrix = if let Some(program) = input.source_transform() {
        if !program.matches_native(scan)? {
            return Err(Error::Gpu(
                "original input differs from captured camera source".into(),
            ));
        }
        program.matrix()
    } else {
        scan.camera_to_ap1
    };
    let node = crate::gpu::source::camera_source(scan)?;
    let photograph = PhotographTaps::authored(limits, node, field, input, matrix, stages)?;
    display_output(photograph.processed, input.luts(), looks, target)
}

pub(crate) type Author =
    dyn Fn(Value, Option<PresentationMapping>) -> Result<Value, Error> + Send + Sync;
type Suffix = fn(Value) -> Result<Value, Error>;

impl Assembly {
    pub(crate) fn authored(
        author: impl Fn(Value, Option<PresentationMapping>) -> Result<Value, Error>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            author: Some(Arc::new(author)),
            ..Self::default()
        }
    }

    pub(crate) fn with_view_pitch(mut self, pitch: Option<f64>) -> Self {
        self.preferred_pitch = pitch;
        self
    }

    pub(crate) fn film(
        settings: crate::photographic::stages::grain::GrainSettings,
        frame: [u32; 2],
        seed: u64,
    ) -> Result<Self, Error> {
        Self::emulation(settings, Default::default(), frame, seed)
    }

    pub(crate) fn emulation(
        film: crate::photographic::stages::grain::GrainSettings,
        halation: crate::photographic::stages::halation::HalationSettings,
        frame: [u32; 2],
        seed: u64,
    ) -> Result<Self, Error> {
        Ok(Self {
            grain: film.parameters(frame)?,
            halation: halation.parameters(frame)?,
            seed,
            ..Self::default()
        })
    }

    pub(crate) fn compose(&self, input: Value) -> Result<Value, Error> {
        self.processed(input, None)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.grain.is_none() && self.halation.is_none() && self.author.is_none()
    }
}

impl std::fmt::Debug for Assembly {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Assembly")
            .field("grain", &self.grain)
            .field("halation", &self.halation)
            .field("seed", &self.seed)
            .field("authored", &self.author.is_some())
            .finish()
    }
}

impl Assembly {
    pub(crate) fn same_authored(&self, other: &Self) -> bool {
        let same_author = match (&self.author, &other.author) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        self.grain == other.grain
            && self.halation == other.halation
            && (self.grain.is_none_or(|grain| grain.amplitude <= 0.) || self.seed == other.seed)
            && same_author
    }
}

thread_local! {
    static SUFFIX: std::cell::Cell<Option<Suffix>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn suffix(image: Value) -> Result<Value, Error> {
    SUFFIX.with(|slot| match slot.get() {
        Some(compose) => compose(image),
        None => Ok(image),
    })
}

pub(crate) fn with_suffix<T>(compose: Suffix, author: impl FnOnce() -> T) -> T {
    struct Restore(Option<Suffix>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SUFFIX.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(SUFFIX.with(|slot| slot.replace(Some(compose))));
    author()
}

#[cfg(any(test, feature = "editor"))]
impl crate::acquisition::Plan {
    pub(crate) fn for_definition(&self, definition: &Assembly) -> Result<Self, Error> {
        Self::for_graph(
            crate::acquisition::Request {
                output: self.output(),
                sampling: self.sampling(),
            },
            |input| definition.sampling_image(input),
        )
    }
}

impl PhotographTaps {
    #[cfg(feature = "project")]
    pub(crate) fn from_recipe(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        recipe: &ImageRecipe,
    ) -> Result<Self, Error> {
        let unrepaired = composition::original_unrepaired(source.clone(), field, recipe)?;
        let neutral =
            crate::gpu::repairs::definition::compose(unrepaired.clone(), &recipe.repairs)?;
        let grading = composition::recipe_grading(limits, recipe, Mapping::Originals)?;
        let source = SourceImages {
            codes: source,
            unrepaired,
        };
        source.finish(neutral, Some(&grading), |graded| {
            composition::film_on_input_grid(graded, recipe)
        })
    }

    pub(crate) fn new(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        input: &AuthoredPhotograph,
        matrix: glam::DMat3,
    ) -> Result<Self, Error> {
        Self::authored(limits, source, field, input, matrix, &Default::default())
    }

    pub(crate) fn authored(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        input: &AuthoredPhotograph,
        matrix: glam::DMat3,
        definition: &Assembly,
    ) -> Result<Self, Error> {
        let (mut assembly, images) = Self::original_fixture(limits, source, field, input, matrix)?;
        assembly.grain = definition.grain;
        assembly.halation = definition.halation;
        assembly.seed = definition.seed;
        assembly.author = definition.author.clone();
        assembly.preferred_pitch = definition.preferred_pitch;
        assembly.photograph(images, Mapping::Originals)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn preview_with_input(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        input: &AuthoredPhotograph,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(Self, crate::acquisition::Plan), Error> {
        let program = input.source_transform().ok_or(Error::InvalidParameter(
            "working photograph has no captured source interpretation",
        ))?;
        if frame.composition.area() != source.domain().finite()?.source()? {
            return Err(Error::InvalidParameter(
                "frame belongs to a different source grid",
            ));
        }
        let (assembly, images) =
            Self::original_fixture(limits, source, field, input, program.matrix())?;
        assembly.preview(images, Mapping::Originals, frame, native_grid)
    }

    fn original_fixture(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        input: &AuthoredPhotograph,
        matrix: glam::DMat3,
    ) -> Result<(Assembly, SourceImages), Error> {
        if source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2]) {
            return Err(Error::InvalidParameter(
                "photographic conversion requires original camera cells",
            ));
        }
        if let Some(grading) = input.grade() {
            grading.require_extent(source.domain().finite()?.source()?)?;
        }
        Self::fixture(
            limits,
            source,
            field,
            Mapping::Originals,
            matrix,
            true,
            input,
        )
    }

    pub(crate) fn from_resident_fixture(
        limits: &wgpu::Limits,
        source: &ProcessingSource,
        input: &AuthoredPhotograph,
        raster: crate::gpu::raster::Raster,
    ) -> Result<Self, Error> {
        input.require_source(source)?;
        if u64::from(raster.size()[0]) * u64::from(raster.size()[1])
            != u64::from(source.pixel_count())
        {
            return Err(Error::InvalidParameter(
                "processing source layout differs from plan",
            ));
        }
        if (!input.repairs.is_empty() || !input.replacements.is_empty())
            && source.geometry().is_none()
        {
            return Err(Error::Gpu(
                "repair rendering needs original source geometry".into(),
            ));
        }
        let domain = Domain {
            bounds: Some(crate::gpu::evaluation::CellRegion::new(
                [0, 0],
                raster.size(),
            )),
            grid: Grid::from_raster(raster),
        };
        let camera = Value::from_operation(
            domain,
            None,
            ResidentCamera {
                domain,
                identity: Arc::downgrade(&source.identity),
                pixels: source.input.clone(),
                continuous: source.continuous,
            },
        );
        let mapping = Mapping::Resident(source.geometry());
        let (assembly, images) = Self::fixture(
            limits,
            camera,
            input.field.clone(),
            mapping,
            source.camera_matrix(),
            source.continuous,
            input,
        )?;
        assembly.photograph(images, mapping)
    }

    fn fixture(
        limits: &wgpu::Limits,
        codes: Value,
        field: Option<Arc<GpuFlatField>>,
        mapping: Mapping,
        matrix: glam::DMat3,
        continuous: bool,
        input: &AuthoredPhotograph,
    ) -> Result<(Assembly, SourceImages), Error> {
        if let Some(program) = input.source_transform() {
            if program.matrix() != matrix {
                return Err(Error::InvalidParameter(
                    "photographic conversion matrix differs from captured input",
                ));
            }
            input.validate_controls(limits)?;
        }

        let interpretation = match input.source_transform() {
            Some(program) => Interpretation::Program(*program),
            None if !input.is_positive() => Interpretation::Packed(input.parameters()),
            None => Interpretation::Unavailable,
        };
        let images =
            SourceImages::from_codes(codes, field, mapping, matrix, continuous, interpretation)?;
        let mut assembly = input.definition();
        if input.parameters().graded() {
            assembly.repairs = input.repairs.clone();
            assembly.replacements = input.replacements.clone();
        }

        if input.parameters().graded() {
            let grading = input
                .grade()
                .ok_or(Error::InvalidParameter(
                    "photographic input requires captured interpretation and grading",
                ))?
                .for_grid(0);
            assembly.grading = Some(match mapping {
                Mapping::Resident(geometry) => grading.with_camera_coordinates(geometry),
                Mapping::Originals => grading,
            });
        }

        Ok((assembly, images))
    }

    pub(crate) fn authored_original(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        program: crate::photographic::stages::interpretation::InputTransform,
        grading: Grading,
        repairs: Vec<crate::dust_repair::Repair>,
        definition: &Assembly,
    ) -> Result<Self, Error> {
        use super::composition::{admit_photograph, require_original_grid};

        require_original_grid(&source, &grading)?;
        admit_photograph(limits, program, &grading)?;
        let images = SourceImages::from_codes(
            source,
            field,
            Mapping::Originals,
            program.matrix(),
            true,
            Interpretation::Program(program),
        )?;
        let fixture = Assembly {
            grading: Some(grading),
            repairs,
            ..Default::default()
        };
        let mut graph = fixture.photograph(images, Mapping::Originals)?;
        graph.processed = definition.compose(graph.processed)?;
        Ok(graph)
    }
}

#[cfg(feature = "editor")]
pub(crate) fn shares_resident_camera(a: &Value, b: &Value) -> bool {
    let cameras = |image: &Value| {
        image
            .ancestors()
            .filter_map(|node| {
                (node.operation().as_ref() as &dyn std::any::Any)
                    .downcast_ref::<ResidentCamera>()
                    .map(|camera| camera.pixels.clone())
            })
            .collect::<Vec<_>>()
    };
    let a = cameras(a);
    let b = cameras(b);
    !a.is_empty() && a.len() == b.len() && a.iter().all(|buffer| b.contains(buffer))
}

#[cfg(feature = "editor")]
pub(crate) fn resident_camera_bytes(image: &Value) -> u64 {
    resident_camera_buffers(image)
        .iter()
        .map(wgpu::Buffer::size)
        .sum()
}

#[cfg(feature = "editor")]
pub(crate) fn has_correction(image: &Value, field: Option<&Arc<GpuFlatField>>) -> bool {
    let mut found = false;
    for node in image.ancestors() {
        if let Some(camera) = node.parameters::<operations::CameraIdentity>() {
            if camera.field.as_ref() != field.map(|field| field.field()) {
                return false;
            }
            found = true;
        }
    }
    found
}

#[cfg(feature = "editor")]
pub(crate) fn filtered(
    grading: &Grading,
    neutral: Value,
    definition: &Assembly,
    presentation: crate::frame_raster::FrameRaster,
    processing: crate::frame_raster::FrameRaster,
) -> Result<Value, Error> {
    let output = composition::PresentationMapping::new(
        presentation,
        processing,
        neutral.domain().grid.coverage,
    );
    definition.processed(grading.compose(neutral)?, Some(output))
}
