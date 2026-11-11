//! Image nodes, operation contracts and declared GPU work. Shader bodies stay opaque to planning.

use super::CellRegion as Rect;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Meaning {
    /// Normalized camera light, before interpretation and the camera-to-working matrix.
    Camera,
    CameraCodes,
    /// Signed native-camera positive, before camera-to-working conversion.
    NativePositive,
    Neutral,
    /// Working AP1 on the pre-inversion source-view branch.
    Inspection,
    Graded,
    /// A scalar mixing weight repeated in RGB, with input metadata retained in w.
    Weight,
    Display,
    /// Packed calculation arguments, not photographic samples on an original grid.
    Arguments,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// Source-coordinate corner of grid cell (0, 0).
    pub origin: [f64; 2],
    pub axes: [[f64; 2]; 2],
    /// Declared source units per grid cell, independent of rounded affine vectors.
    pub pitch: [f64; 2],
    pub coverage: u32,
}

impl Grid {
    pub fn axis_aligned(origin: [f64; 2], pitch: [f64; 2]) -> Self {
        Self {
            origin,
            axes: [[pitch[0], 0.], [0., pitch[1]]],
            pitch,
            coverage: 0,
        }
    }

    pub fn point(self, p: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|i| self.origin[i] + p[0] * self.axes[0][i] + p[1] * self.axes[1][i])
    }

    pub fn from_frame(frame: crate::frame_raster::FrameRaster) -> Self {
        let origin = frame.source_point([0.; 2]).0;
        let axes = [[1., 0.], [0., 1.]].map(|point| {
            let next = frame.source_point(point).0;
            std::array::from_fn(|i| next[i] - origin[i])
        });
        Self {
            origin,
            axes,
            pitch: std::array::from_fn(|i| {
                (frame.max.0[i] - frame.min.0[i]) / f64::from(frame.output[i])
            }),
            coverage: 0,
        }
    }

    pub(crate) fn from_raster(raster: crate::gpu::raster::Raster) -> Self {
        let axes = [
            raster.axis_x[..2].try_into().unwrap(),
            raster.axis_y[..2].try_into().unwrap(),
        ]
        .map(|axis: [f32; 2]| axis.map(f64::from));
        Self {
            origin: std::array::from_fn(|i| {
                f64::from(raster.origin[i]) - 0.5 * (axes[0][i] + axes[1][i])
            }),
            axes,
            pitch: raster.pitch().map(f64::from),
            coverage: raster.dimensions[2],
        }
    }

    pub(crate) fn raster(self, region: Rect) -> crate::gpu::raster::Raster {
        let origin = self.point([f64::from(region.x) + 0.5, f64::from(region.y) + 0.5]);
        crate::gpu::raster::Raster {
            dimensions: [region.width, region.height, self.coverage, 0],
            origin: [origin[0] as f32, origin[1] as f32, 0., 0.],
            axis_x: [
                self.axes[0][0] as f32,
                self.axes[0][1] as f32,
                self.pitch[0] as f32,
                0.,
            ],
            axis_y: [
                self.axes[1][0] as f32,
                self.axes[1][1] as f32,
                self.pitch[1] as f32,
                0.,
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Domain {
    /// Finite source-cell boundaries; reconstructed planes have no new viewport edge.
    pub bounds: Option<Rect>,
    pub grid: Grid,
    pub meaning: Meaning,
}

impl Domain {
    pub fn contains_region(self, region: Rect) -> bool {
        region.validate().is_ok()
            && self
                .bounds
                .is_none_or(|bounds| bounds.contains_region(region))
    }

    pub fn finite(self) -> Result<Rect, Error> {
        self.bounds.ok_or(Error::InvalidParameter(
            "image has no finite sample boundary",
        ))
    }

    pub fn point(self, p: [f64; 2]) -> [f64; 2] {
        self.grid.point(p)
    }

    pub fn same_grid(self, other: Self) -> bool {
        self.grid == other.grid
    }
}

/// Immutable source samples on a captured domain.
pub trait Source: Send + Sync {
    fn domain(&self) -> Domain;

    fn identity(&self) -> Option<RetainedKey> {
        None
    }

    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error>;

    fn storage_format(&self) -> ImageFormat {
        ImageFormat::Rgba32Float
    }

    fn read_codes(&self, _region: Rect) -> Result<Vec<[u16; 4]>, Error> {
        Err(Error::InvalidParameter(
            "source does not provide native camera codes",
        ))
    }
}

/// One logical input region, identified by its position in the node's upstream calculations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    pub input: usize,
    pub region: Rect,
}

impl Read {
    pub fn new(input: usize, region: Rect) -> Self {
        Self { input, region }
    }
}

/// Declares logical upstream reads and work for a captured image operation.
pub trait Operation: std::any::Any + Send + Sync {
    /// Lower a captured mathematical definition to unit-local compiler expressions.
    fn calculation(
        self: std::sync::Arc<Self>,
        _inputs: &[crate::gpu::evaluation::value::Value],
        _domain: Domain,
        _output: Rect,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        Ok(None)
    }

    fn access(&self) -> value::Access {
        value::Access::Pointwise
    }

    fn reads(&self, _output: Rect) -> Result<Vec<Read>, Error> {
        Err(Error::InvalidParameter(
            "GPU primitive does not declare its inputs",
        ))
    }

    fn work_units(&self, output: Rect, _budget: Budget) -> Result<WorkUnits, Error> {
        output.validate()?;
        // Ordinary GPU operations accept any admitted rectangle, including a consumer's halo.
        // Stages with intrinsic or streamed units override this decomposition.
        Ok(Box::new(std::iter::once(output)))
    }

    fn source(&self) -> Option<Arc<dyn Source>> {
        None
    }

    /// Mathematical arguments, independent of execution and physical storage.
    fn key(&self) -> Option<RetainedKey> {
        None
    }

    /// Describe content over the requested cells without expanding executable calculations.
    /// Disjoint requests may omit cells inside the enclosing output rectangle.
    fn describe(
        &self,
        output: Output,
        _inputs: &[crate::gpu::evaluation::value::Value],
        _requested: &[Rect],
    ) -> Result<Output, Error> {
        Ok(output)
    }
}

/// A requested mathematical result, without executable instructions or pixel ownership.
#[derive(Clone)]
pub struct Output {
    pub domain: Domain,
    pub region: Rect,
    pub(super) content: Arc<Content>,
}
impl Output {
    pub fn identity(&self) -> Option<&RetainedKey> {
        self.content.identity.as_ref()
    }

    pub fn same_value(&self, other: &Self) -> bool {
        self.content.same(&other.content)
    }

    pub(crate) fn content(&self) -> Arc<Content> {
        self.content.clone()
    }

    pub(crate) fn calculated(
        domain: Domain,
        region: Rect,
        key: Option<RetainedKey>,
        inputs: impl IntoIterator<Item = Arc<Content>>,
    ) -> Self {
        Self {
            domain,
            region,
            content: Content::new(domain, key, None, inputs),
        }
    }
}
impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output")
            .field("domain", &self.domain)
            .field("region", &self.region)
            .field("identity", &self.identity())
            .finish()
    }
}
impl PartialEq for Output {
    fn eq(&self, other: &Self) -> bool {
        self.region == other.region && self.same_value(other)
    }
}

/// Mathematical content has no producer callback or physical allocation.
pub(crate) struct Content {
    pub(super) domain: Domain,
    pub(super) key: Option<RetainedKey>,
    capture: Option<value::Capture>,
    inputs: Vec<Arc<Content>>,
    pub(super) identity: Option<RetainedKey>,
}
impl Content {
    pub(super) fn new(
        domain: Domain,
        key: Option<RetainedKey>,
        capture: Option<value::Capture>,
        inputs: impl IntoIterator<Item = Arc<Content>>,
    ) -> Arc<Self> {
        let inputs: Vec<_> = inputs.into_iter().collect();
        let identity = key.clone().and_then(|operation| {
            Some(RetainedKey::new(ImageIdentity {
                domain,
                operation,
                upstream: inputs
                    .iter()
                    .map(|input| input.identity.clone())
                    .collect::<Option<Vec<_>>>()?,
            }))
        });
        let capture = key.is_none().then_some(capture).flatten();
        Arc::new(Self {
            domain,
            key,
            capture,
            inputs,
            identity,
        })
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        if let (Some(a), Some(b)) = (&self.identity, &other.identity) {
            return a == b;
        }
        self.domain == other.domain
            && match (&self.key, &other.key) {
                (Some(a), Some(b)) => a == b,
                (None, None) => self
                    .capture
                    .as_ref()
                    .zip(other.capture.as_ref())
                    .is_some_and(|(a, b)| a.same(b)),
                _ => false,
            }
            && self.inputs.len() == other.inputs.len()
            && self
                .inputs
                .iter()
                .zip(&other.inputs)
                .all(|(a, b)| a.same(b))
    }
}
impl Drop for Content {
    fn drop(&mut self) {
        // Children still own their keys while this parent releases its upstream key references.
        drop(self.identity.take());
        let mut pending = std::mem::take(&mut self.inputs);
        while let Some(input) = pending.pop() {
            if let Some(mut input) = Arc::into_inner(input) {
                drop(input.identity.take());
                pending.append(&mut input.inputs);
            }
        }
    }
}

#[derive(Debug, PartialEq)]
struct ImageIdentity {
    domain: Domain,
    upstream: Vec<RetainedKey>,
    operation: RetainedKey,
}

/// Pointwise ABI: fn(vec4f, vec2f) -> vec4f, preserving metadata in w.
/// Module symbols and bindings are private to the function. Evaluation links their namespaces
/// when composing calculations, whether the input is stored or callable.
#[derive(Clone)]
pub struct Function {
    pub code: Arc<str>,
    pub symbol: &'static str,
    pub bindings: Vec<(u32, FunctionBinding)>,
}

impl Function {
    pub fn new(code: impl Into<Arc<str>>, symbol: &'static str) -> Self {
        Self {
            code: code.into(),
            symbol,
            bindings: vec![],
        }
    }

    pub fn with_shared(mut self, slot: u32, resource: Shared) -> Self {
        self.bindings
            .push((slot, FunctionBinding::Shared(resource)));
        self
    }

    /// Capture parameter bytes without a device or shader specialization. Planning owns their
    /// uniform allocation and admission, just like other declared operation constants.
    /// The value's byte layout must match the shader's WGSL uniform declaration.
    pub fn with_uniform<T: bytemuck::Pod>(mut self, slot: u32, value: &T) -> Self {
        self.bindings.push((
            slot,
            FunctionBinding::Uniform(bytemuck::bytes_of(value).into()),
        ));
        self
    }

    /// Capture immutable storage bytes; their allocation follows the consuming plan.
    pub fn with_storage(mut self, slot: u32, bytes: impl Into<Arc<[u8]>>) -> Self {
        self.bindings
            .push((slot, FunctionBinding::Storage(bytes.into())));
        self
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum FunctionBinding {
    Shared(Shared),
    Uniform(Arc<[u8]>),
    Storage(Arc<[u8]>),
}

impl From<Shared> for FunctionBinding {
    fn from(value: Shared) -> Self {
        Self::Shared(value)
    }
}

impl FunctionBinding {
    pub(crate) fn lower(&self, fragment: &mut Fragment) -> Binding {
        match self {
            Self::Shared(value) => Binding::Shared(value.clone()),
            Self::Uniform(bytes) => {
                Binding::Uniform(fragment.resource(Storage::Uniform(bytes.clone())))
            }
            Self::Storage(bytes) => Binding::Read(fragment.resource(Storage::Data(bytes.clone()))),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Shared {
    Buffer(wgpu::Buffer),
    Texture(wgpu::TextureView),
    StorageTexture(wgpu::TextureView),
    Sampler(wgpu::Sampler),
    Image {
        image: Arc<ImageValue>,
        view: Option<wgpu::TextureView>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resource(pub usize);

/// A demanded region and symbolic resource within a plan, rather than an image expression.
#[derive(Clone, Copy, PartialEq)]
pub struct PlannedImage {
    pub resource: Resource,
    pub domain: Domain,
    pub region: Rect,
}

impl PlannedImage {
    pub fn size(self) -> [u32; 2] {
        [self.region.width, self.region.height]
    }

    pub fn cells(self) -> u32 {
        self.region.width * self.region.height
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Storage {
    Input {
        slot: usize,
        bytes: u64,
        writable: bool,
    },
    /// A caller-owned read-only image; its physical owner survives recorded consumers.
    ResidentBuffer(Arc<ImageValue>),
    Buffer(u64),
    Uniform(Arc<[u8]>),
    Data(Arc<[u8]>),
    Image {
        key: RetainedKey,
        layout: ImageLayout,
        pixels: Option<Arc<ImageValue>>,
    },
    Texture([u32; 2]),
    /// Caller-owned RGBA32Float attachment; lifetime follows recorded commands.
    RenderTarget(wgpu::Texture),
}

impl Storage {
    pub(crate) fn read_only(&self) -> bool {
        matches!(
            self,
            Self::Input {
                writable: false,
                ..
            } | Self::ResidentBuffer(_)
                | Self::Image {
                    pixels: Some(_),
                    ..
                }
        )
    }
    pub(crate) fn starts_initialized(&self) -> bool {
        matches!(
            self,
            Self::Uniform(_)
                | Self::Data(_)
                | Self::Input { .. }
                | Self::ResidentBuffer(_)
                | Self::Image {
                    pixels: Some(_),
                    ..
                }
        )
    }

    pub fn bytes(&self) -> u64 {
        match self {
            Self::Input { .. } | Self::ResidentBuffer(_) | Self::RenderTarget(_) => 0,
            Self::Buffer(bytes) => *bytes,
            Self::Image { layout, .. } => layout.bytes(),
            Self::Uniform(bytes) | Self::Data(bytes) => bytes.len() as u64,
            Self::Texture(size) => u64::from(size[0]) * u64::from(size[1]) * 16,
        }
    }

    pub fn can_hold(&self, next: &Self) -> bool {
        match (self, next) {
            (
                Self::Input {
                    bytes,
                    writable: true,
                    ..
                },
                Self::Buffer(next),
            ) => bytes >= next,
            (Self::Buffer(a), Self::Buffer(b)) => a >= b,
            (Self::Texture(a), Self::Texture(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Binding {
    Read(Resource),
    Write(Resource),
    Uniform(Resource),
    Texture(Resource),
    Shared(Shared),
}

impl Binding {
    pub fn local(&self) -> Option<Resource> {
        match self {
            Self::Read(r) | Self::Write(r) | Self::Uniform(r) | Self::Texture(r) => Some(*r),
            Self::Shared(_) => None,
        }
    }
}

#[derive(Clone)]
pub enum RenderProgram {
    Captured {
        pipeline: wgpu::RenderPipeline,
        layout: wgpu::BindGroupLayout,
    },
    Shader(Arc<str>),
}

#[derive(Clone)]
pub enum Work {
    Produce {
        image: value::Value,
        output: PlannedImage,
    },
    Upload {
        source: Arc<dyn Source>,
        output: PlannedImage,
    },
    Copy {
        source: Resource,
        target: Resource,
        source_offset: u64,
        target_offset: u64,
        bytes: u64,
    },
    Compute {
        code: Arc<str>,
        entry: &'static str,
        bindings: Vec<(u32, Binding)>,
        groups: [u32; 3],
        initialize: Option<Resource>,
    },
    Render {
        program: RenderProgram,
        bindings: Vec<(u32, Binding)>,
        target: Resource,
    },
}

impl Work {
    pub(crate) fn linear(
        code: impl Into<Arc<str>>,
        entry: &'static str,
        bindings: Vec<(u32, Binding)>,
        cells: u32,
    ) -> Self {
        let groups = cells.div_ceil(64);
        let width = groups.min(wgpu::Limits::default().max_compute_workgroups_per_dimension);
        Self::Compute {
            code: code.into(),
            entry,
            bindings,
            groups: [width, groups.div_ceil(width.max(1)), 1],
            initialize: None,
        }
    }

    pub fn resources(&self) -> Vec<Resource> {
        match self {
            Self::Upload { output, .. } | Self::Produce { output, .. } => vec![output.resource],
            Self::Copy { source, target, .. } => vec![*source, *target],
            Self::Compute { bindings, .. } => {
                bindings.iter().filter_map(|(_, b)| b.local()).collect()
            }
            Self::Render {
                bindings, target, ..
            } => bindings
                .iter()
                .filter_map(|(_, b)| b.local())
                .chain([*target])
                .collect(),
        }
    }
}

#[derive(Clone, Default)]
pub struct Fragment {
    programs: Option<Arc<Programs>>,
    pub resources: Vec<Storage>,
    pub work: Vec<Work>,
}

impl Fragment {
    pub(super) fn with_programs(programs: Option<Arc<Programs>>) -> Self {
        Self {
            programs,
            ..Self::default()
        }
    }

    pub(super) fn library(&self, code: &str) -> Result<Arc<sampling::Library>, Error> {
        match &self.programs {
            Some(programs) => programs.libraries.get(code),
            None => sampling::Library::parse(code).map(Arc::new),
        }
    }

    pub fn resource(&mut self, storage: Storage) -> Resource {
        if let Storage::Uniform(bytes) = &storage
            && let Some(index) = self
                .resources
                .iter()
                .position(|existing| matches!(existing,Storage::Uniform(other) if other==bytes))
        {
            return Resource(index);
        }
        let resource = Resource(self.resources.len());
        self.resources.push(storage);
        resource
    }

    pub fn image(&mut self, domain: Domain, region: Rect) -> PlannedImage {
        PlannedImage {
            resource: self.resource(Storage::Buffer(
                u64::from(region.width) * u64::from(region.height) * 16,
            )),
            domain,
            region,
        }
    }

    pub fn uniform<T: bytemuck::Pod>(&mut self, value: &T) -> Binding {
        Binding::Uniform(self.resource(Storage::Uniform(bytemuck::bytes_of(value).into())))
    }

    pub fn initialize(&mut self, resource: Resource) {
        let Some(Work::Compute { initialize, .. }) = self.work.last_mut() else {
            panic!("an initializer must be compute work");
        };
        *initialize = Some(resource);
    }

    /// Linear work with 64-thread groups. Kernels flatten the invocation ID as
    /// `id.x + id.y * num_workgroups.x * 64`, including the bounds check.
    pub fn compute(
        &mut self,
        code: impl Into<Arc<str>>,
        entry: &'static str,
        bindings: Vec<(u32, Binding)>,
        cells: u32,
    ) {
        let initialize = bindings.iter().find_map(|(_, binding)| match binding {
            Binding::Write(resource)
                if matches!(self.resources[resource.0], Storage::Image { .. }) =>
            {
                Some(*resource)
            }
            _ => None,
        });
        let mut work = Work::linear(code, entry, bindings, cells);
        if let Work::Compute {
            initialize: target, ..
        } = &mut work
        {
            *target = initialize;
        }
        self.work.push(work);
    }
}
