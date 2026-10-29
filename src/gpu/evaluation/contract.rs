//! Image nodes, operation contracts and declared GPU work. Shader bodies stay opaque to planning.
use super::*;
use crate::geometry::SourceRegion as Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Meaning {
    /// Normalized continuous camera light, before the camera-to-working matrix.
    #[cfg(all(test, feature = "editor"))]
    Camera,
    CameraCodes,
    Neutral,
    Graded,
    Display,
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
    pub bounds: Rect,
    pub grid: Grid,
    pub meaning: Meaning,
}
impl Domain {
    pub fn point(self, p: [f64; 2]) -> [f64; 2] {
        self.grid.point(p)
    }
    pub fn same_grid(self, other: Self) -> bool {
        self.bounds == other.bounds && self.grid == other.grid
    }
}

pub trait Source: Send + Sync {
    fn domain(&self) -> Domain;
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error>;
}

/// Declares upstream demand and GPU lowering for a captured image operation.
pub trait Operation: Send + Sync {
    fn input(&self, output: Rect) -> Result<Option<Rect>, Error>;
    fn pointwise(&self) -> Option<Function> {
        None
    }
    fn in_place(&self) -> bool {
        false
    }
    fn sample_producer(&self) -> Option<&dyn SampleProducer> {
        None
    }
    fn sample_consumer(&self) -> Option<&dyn SampleConsumer> {
        None
    }
    fn lower(
        &self,
        fragment: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error>;
}

/// An addressable result: its domain, upstream image and captured producing operation.
/// Nodes describe composition, not mandatory buffers or dispatches.
pub struct ImageNode {
    pub domain: Domain,
    pub input: Option<Arc<ImageNode>>,
    pub operation: Arc<dyn Operation>,
    pub(crate) implementation: Option<Arc<ImageNode>>,
}
impl ImageNode {
    pub fn new(
        domain: Domain,
        input: Option<Arc<ImageNode>>,
        operation: impl Operation + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            domain,
            input,
            operation: Arc::new(operation),
            implementation: None,
        })
    }
    /// Supply an equivalent physical implementation without erasing logical image boundaries.
    /// Composed planning selects it; materialized planning follows the logical chain.
    /// Approximate preview orderings are separate compositions, not equivalent implementations.
    pub fn implemented_by(
        logical: Arc<Self>,
        implementation: Arc<Self>,
    ) -> Result<Arc<Self>, Error> {
        if logical.domain != implementation.domain {
            return Err(Error::InvalidParameter(
                "equivalent implementation changes the image domain",
            ));
        }
        Ok(Arc::new(Self {
            domain: logical.domain,
            input: logical.input.clone(),
            operation: logical.operation.clone(),
            implementation: Some(implementation),
        }))
    }
}

/// Pointwise ABI: fn(vec4f, vec2f) -> vec4f, preserving metadata in w.
/// Modules own bindings below 20; 20–22 belong to the map entrypoint. Functions from the
/// same module may share identical resources. Conflicting bindings force materialization.
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
    pub(crate) fn lower_bindings(&self, fragment: &mut Fragment) -> Vec<(u32, Binding)> {
        self.bindings
            .iter()
            .map(|(slot, value)| (*slot, value.lower(fragment)))
            .collect()
    }
}
#[derive(Clone, PartialEq, Eq)]
pub enum FunctionBinding {
    Shared(Shared),
    Uniform(Arc<[u8]>),
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
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Shared {
    Buffer(wgpu::Buffer),
    Texture(wgpu::TextureView),
    StorageTexture(wgpu::TextureView),
    Sampler(wgpu::Sampler),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resource(pub usize);
/// A demanded region and symbolic resource within a plan, rather than an image expression.
#[derive(Clone, Copy)]
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
    Buffer(u64),
    Uniform(Arc<[u8]>),
    Data(Arc<[u8]>),
    Retained {
        key: RetainedKey,
        bytes: u64,
    },
    Texture([u32; 2]),
    /// Caller-owned RGBA32Float attachment; lifetime follows recorded commands.
    RenderTarget(wgpu::Texture),
}
impl Storage {
    pub fn bytes(&self) -> u64 {
        match self {
            Self::Input { .. } | Self::RenderTarget(_) => 0,
            Self::Buffer(bytes) | Self::Retained { bytes, .. } => *bytes,
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

#[derive(Clone)]
pub enum Binding {
    Input { slot: usize, bytes: u64 },
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
            Self::Shared(_) | Self::Input { .. } => None,
        }
    }
}

pub enum Work {
    Upload {
        source: Arc<dyn Source>,
        output: PlannedImage,
    },
    Compute {
        code: Arc<str>,
        entry: &'static str,
        bindings: Vec<(u32, Binding)>,
        groups: [u32; 3],
        initialize: Option<Resource>,
    },
    Render {
        pipeline: wgpu::RenderPipeline,
        layout: wgpu::BindGroupLayout,
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
            Self::Upload { output, .. } => vec![output.resource],
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

#[derive(Default)]
pub struct Fragment {
    pub resources: Vec<Storage>,
    pub work: Vec<Work>,
}
impl Fragment {
    pub fn resource(&mut self, storage: Storage) -> Resource {
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
    pub fn retained(&mut self, key: RetainedKey, bytes: u64) -> Resource {
        self.resource(Storage::Retained { key, bytes })
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
        self.work.push(Work::linear(code, entry, bindings, cells));
    }
}
