//! Layer contracts and declarative GPU work. Shader bodies are opaque to demand planning.
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

/// A single-input image operation; no branching graph or plugin registry in this slice.
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
        input: Option<Image>,
        output: Image,
    ) -> Result<(), Error>;
}

pub struct Layer {
    pub domain: Domain,
    pub input: Option<Arc<Layer>>,
    pub operation: Arc<dyn Operation>,
}
impl Layer {
    pub fn new(
        domain: Domain,
        input: Option<Arc<Layer>>,
        operation: impl Operation + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            domain,
            input,
            operation: Arc::new(operation),
        })
    }
}

/// Pointwise ABI: fn(vec4f, vec2f) -> vec4f, preserving metadata in w.
/// Modules own bindings below 20; 20–22 belong to the map entrypoint. Binding collisions
/// force materialization rather than silently rebinding a different operation's constants.
#[derive(Clone)]
pub struct Function {
    pub code: Arc<str>,
    pub symbol: &'static str,
    pub bindings: Vec<(u32, Shared)>,
}

#[derive(Clone)]
pub enum Shared {
    Buffer(wgpu::Buffer),
    Texture(wgpu::TextureView),
    Sampler(wgpu::Sampler),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resource(pub usize);
#[derive(Clone, Copy)]
pub struct Image {
    pub resource: Resource,
    pub domain: Domain,
    pub region: Rect,
}
impl Image {
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
}
impl Storage {
    pub fn bytes(&self) -> u64 {
        match self {
            Self::Input { .. } => 0,
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
        output: Image,
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
    pub fn image(&mut self, domain: Domain, region: Rect) -> Image {
        Image {
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
        let groups = cells.div_ceil(64);
        let width = groups.min(wgpu::Limits::default().max_compute_workgroups_per_dimension);
        self.work.push(Work::Compute {
            code: code.into(),
            entry,
            bindings,
            groups: [width, groups.div_ceil(width.max(1)), 1],
            initialize: None,
        });
    }
}
