//! Fixture graph authoring uses the ordinary operation functions.
use super::{assembly::Assembly, composition::ViewOutput, *};

#[cfg(all(feature = "raw", feature = "render", feature = "editor"))]
pub(crate) fn original_chain(
    limits: &wgpu::Limits,
    scan: &crate::scan::Scan,
    field: Option<Arc<GpuFlatField>>,
    input: &crate::gpu::test_inputs::AuthoredPhotograph,
    stages: &Assembly,
    looks: &crate::gpu::lut3d::Looks,
    target: &crate::gpu::drt::Target,
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
        program.matrix
    } else {
        scan.camera_to_ap1
    };
    let node = camera_source(scan)?;
    let photograph = PhotographGraph::authored(limits, node, field, input, matrix, stages)?;
    display_output(photograph.processed, input.luts(), looks, target)
}

pub(crate) type Author = dyn Fn(Value, Option<ViewOutput>) -> Result<Value, Error> + Send + Sync;
type Suffix = fn(Value) -> Result<Value, Error>;

impl Assembly {
    pub(crate) fn authored(
        author: impl Fn(Value, Option<ViewOutput>) -> Result<Value, Error> + Send + Sync + 'static,
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
        settings: crate::grain::GrainSettings,
        frame: [u32; 2],
        seed: u64,
    ) -> Result<Self, Error> {
        Self::emulation(settings, Default::default(), frame, seed)
    }

    pub(crate) fn emulation(
        film: crate::grain::GrainSettings,
        halation: crate::halation::HalationSettings,
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

impl crate::image_mtf::ImageMtfParameters {
    pub(crate) fn compose(&self, input: Value) -> Result<Value, Error> {
        crate::gpu::gaussian::blur(input, self.sigma)
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

impl PhotographGraph {
    pub(crate) fn authored_original(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        program: crate::gpu::interpretation::InputTransform,
        grading: Grading,
        repairs: Vec<crate::dust_repair::Repair>,
        definition: &Assembly,
    ) -> Result<Self, Error> {
        let mut graph = Self::original(limits, source, field, program, grading, repairs)?;
        graph.processed = definition.compose(graph.processed)?;
        Ok(graph)
    }
}
