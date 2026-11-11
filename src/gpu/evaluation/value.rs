//! Immutable mathematical values and captured calculation bodies.
use super::*;
mod definition;
pub(super) use definition::Definition;

mod applied;
mod calculation;
mod compiler;
pub(crate) use calculation::Calculation;
mod graph;
mod nodes;
pub(in crate::gpu::evaluation) use graph::{FunctionGraph, Kernel as CompiledKernel};
pub(crate) use graph::{MAX_UNIT_VALUES, compile as compile_gpu_graph, retire as retire_gpu_graph};
#[cfg(test)]
pub(crate) mod testing;

pub use nodes::{gaussian, pointwise, source};

#[cfg(all(test, feature = "editor"))]
mod tests;

/// An identified calculation and its requested coverage, not a pixel allocation.
#[derive(Clone)]
pub struct Value {
    definition: Arc<Definition>,
    demand: Option<Output>,
    access: Access,
}

/// A weak calculation handle; descriptions do not retain physical producer resources.
#[derive(Clone)]
pub(crate) struct WeakValue {
    definition: std::sync::Weak<Definition>,
    demand: Option<Output>,
    access: Access,
}

impl WeakValue {
    pub(crate) fn upgrade(&self) -> Option<Value> {
        Some(Value {
            definition: self.definition.upgrade()?,
            demand: self.demand.clone(),
            access: self.access,
        })
    }
}

impl Value {
    pub(crate) fn downgrade(&self) -> WeakValue {
        WeakValue {
            definition: Arc::downgrade(&self.definition),
            demand: self.demand.clone(),
            access: self.access,
        }
    }
}

/// Input access used by the compiler's conservative fusion rule.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Each image input is read at the output position.
    Pointwise,
    /// An image input can be read at other positions in its declared coverage.
    Neighborhood,
}

/// One calculation body; the executor owns input acquisition and physical output.
#[derive(Clone)]
pub struct Operation {
    key: Option<RetainedKey>,
    body: Arc<Body>,
}

#[derive(Clone)]
enum Body {
    Source(Arc<dyn super::Source>),
    Argument { writable: bool },
    Gpu(GpuBody),
    Cpu(Arc<dyn CpuKernel>),
    Concatenate,
    Failed(Error),
}

#[derive(Clone)]
struct GpuBody {
    code: Arc<str>,
    entry: &'static str,
    uniform: Option<(&'static str, Arc<[u8]>)>,
    tables: Vec<Arc<[[u32; 4]]>>,
    bindings: Vec<(u32, FunctionBinding)>,
    access: Access,
    input_access: Vec<Access>,
    shape: Shape,
    sampling_work: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Pixel,
    Pair,
}
impl Shape {
    fn wgsl(self) -> &'static str {
        match self {
            Self::Pixel => "vec4f",
            Self::Pair => "array<vec4f, 2>",
        }
    }
    fn bytes(self) -> u64 {
        match self {
            Self::Pixel => 16,
            Self::Pair => 32,
        }
    }
}

#[derive(Debug, PartialEq)]
struct Arguments {
    contract: &'static str,
    parameters: RetainedKey,
}

impl Operation {
    pub(super) fn argument(writable: bool) -> Self {
        Self {
            key: None,
            body: Arc::new(Body::Argument { writable }),
        }
    }
    /// `entry` returns vec4f from cell/world position, current input pixels, and optional
    /// parameters. Neighborhood bodies instead access declared images through input_i(p).
    /// The contract identifies the mathematics, independently of generated shader code.
    pub fn new<T: std::fmt::Debug + PartialEq + Send + Sync + 'static>(
        contract: &'static str,
        parameters: T,
        code: impl Into<Arc<str>>,
        entry: &'static str,
        access: Access,
    ) -> Self {
        Self {
            key: Some(RetainedKey::new(Arguments {
                contract,
                parameters: RetainedKey::new(parameters),
            })),
            body: Arc::new(Body::Gpu(GpuBody {
                code: code.into(),
                entry,
                sampling_work: 1,
                uniform: None,
                tables: Vec::new(),
                bindings: Vec::new(),
                access,
                input_access: Vec::new(),
                shape: Shape::Pixel,
            })),
        }
    }

    /// Captured shader-library arguments are linked privately with the calculation.
    pub fn with_bindings(mut self, bindings: Vec<(u32, FunctionBinding)>) -> Self {
        let Body::Gpu(body) = Arc::make_mut(&mut self.body) else {
            panic!("only GPU calculations bind shader resources");
        };
        body.bindings = bindings;
        self
    }

    pub(crate) fn pointwise_function<T: std::fmt::Debug + PartialEq + Send + Sync + 'static>(
        contract: &'static str,
        parameters: T,
        function: Function,
    ) -> Self {
        Self::shader_function(
            Some(RetainedKey::new(Arguments {
                contract,
                parameters: RetainedKey::new(parameters),
            })),
            function,
        )
    }

    pub(crate) fn shader_function(key: Option<RetainedKey>, function: Function) -> Self {
        let code = format!(
            "{}\nfn value_pixel(cell:vec2i, position:vec2f, pixel:vec4f)->vec4f {{ if pixel.w < 0.0 {{ return pixel; }} return {}(pixel,position); }}",
            function.code, function.symbol
        );
        Self {
            key,
            body: Arc::new(Body::Gpu(GpuBody {
                code: code.into(),
                entry: "value_pixel",
                sampling_work: 1,
                uniform: None,
                tables: Vec::new(),
                bindings: function.bindings,
                access: Access::Pointwise,
                input_access: Vec::new(),
                shape: Shape::Pixel,
            })),
        }
    }

    /// A declared upper bound for sampling work in one output invocation.
    pub(crate) fn with_sampling_work(mut self, work: u64) -> Self {
        let Body::Gpu(body) = Arc::make_mut(&mut self.body) else {
            panic!("sampling work belongs to GPU calculations");
        };
        body.sampling_work = work;
        self
    }

    pub(crate) fn with_shape(mut self, shape: Shape) -> Self {
        let Body::Gpu(body) = Arc::make_mut(&mut self.body) else {
            panic!("CPU image kernels return pixels");
        };
        body.shape = shape;
        self
    }

    /// A whole CPU calculation receives its declared image values, not an evaluator.
    pub fn cpu<T: std::fmt::Debug + PartialEq + Send + Sync + 'static>(
        contract: &'static str,
        parameters: T,
        kernel: Arc<dyn CpuKernel>,
    ) -> Self {
        Self {
            key: Some(RetainedKey::new(Arguments {
                contract,
                parameters: RetainedKey::new(parameters),
            })),
            body: Arc::new(Body::Cpu(kernel)),
        }
    }

    pub(crate) fn failed<T: std::fmt::Debug + PartialEq + Send + Sync + 'static>(
        contract: &'static str,
        parameters: T,
        error: Error,
    ) -> Self {
        Self {
            key: Some(RetainedKey::new(Arguments {
                contract,
                parameters: RetainedKey::new(parameters),
            })),
            body: Arc::new(Body::Failed(error)),
        }
    }

    /// Mixed bodies receive pointwise pixels and use input_i for neighborhood inputs.
    pub fn with_input_access(mut self, access: Vec<Access>) -> Self {
        let Body::Gpu(body) = Arc::make_mut(&mut self.body) else {
            panic!("CPU calculations consume complete declared inputs");
        };
        body.input_access = access;
        self
    }

    fn access(&self) -> Access {
        match self.body.as_ref() {
            Body::Source(_) | Body::Argument { .. } => Access::Pointwise,
            Body::Gpu(body) => body.access,
            Body::Cpu(_) | Body::Concatenate | Body::Failed(_) => Access::Neighborhood,
        }
    }

    fn input_access(&self, index: usize) -> Access {
        match self.body.as_ref() {
            Body::Source(_) | Body::Argument { .. } => Access::Pointwise,
            Body::Gpu(body) => body.input_access.get(index).copied().unwrap_or(body.access),
            Body::Cpu(_) | Body::Concatenate | Body::Failed(_) => Access::Neighborhood,
        }
    }

    /// An immutable integer table is accessed through table_i(index) in the WGSL body.
    pub fn with_table(mut self, rows: impl Into<Arc<[[u32; 4]]>>) -> Self {
        let Body::Gpu(body) = Arc::make_mut(&mut self.body) else {
            panic!("CPU calculations do not have GPU tables");
        };
        body.tables.push(rows.into());
        self
    }

    /// Capture the packed GPU representation separately from mathematical parameters.
    pub fn with_uniform<T: bytemuck::Pod>(mut self, ty: &'static str, value: &T) -> Self {
        let Body::Gpu(body) = Arc::make_mut(&mut self.body) else {
            panic!("CPU calculations do not have GPU uniforms");
        };
        body.uniform = Some((ty, bytemuck::bytes_of(value).into()));
        self
    }
}

pub(super) struct Capture {
    source: Option<std::sync::Weak<dyn Source>>,
    body: Option<std::sync::Weak<Body>>,
}
impl Capture {
    pub(super) fn same(&self, other: &Self) -> bool {
        self.source
            .as_ref()
            .zip(other.source.as_ref())
            .is_some_and(|(a, b)| std::sync::Weak::ptr_eq(a, b))
            || self
                .body
                .as_ref()
                .zip(other.body.as_ref())
                .is_some_and(|(a, b)| std::sync::Weak::ptr_eq(a, b))
    }
}
pub(super) fn capture(node: &dyn super::Operation) -> Option<Capture> {
    if let Some(source) = node.source() {
        return Some(Capture {
            source: Some(Arc::downgrade(&source)),
            body: None,
        });
    }
    fn operation(node: &dyn super::Operation) -> Option<&Operation> {
        let node = node as &dyn std::any::Any;
        node.downcast_ref::<applied::Applied>()
            .map(|node| &node.operation)
            .or_else(|| node.downcast_ref::<nodes::Pointwise>().map(|node| &node.0))
            .or_else(|| {
                node.downcast_ref::<super::resident::Resident>()
                    .map(|node| &node.0)
            })
    }
    operation(node).map(|operation| Capture {
        source: None,
        body: Some(Arc::downgrade(&operation.body)),
    })
}

pub(super) fn constituents(calculation: &Calculation) -> Result<Option<Vec<Value>>, Error> {
    if !calculation.primitive().is_some_and(|primitive| {
        matches!(
            primitive.operation.body.as_ref(),
            Body::Gpu(_) | Body::Concatenate
        )
    }) {
        return Ok(None);
    }
    let mut boundaries = Vec::new();
    let mut boundary = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut pending = vec![calculation.clone()];
    let mut count = 0;
    while let Some(node) = pending.pop() {
        if !seen.insert(node.pointer()) {
            continue;
        }
        if let Some(value) = node.value() {
            if boundary.insert(value.pointer()) {
                boundaries.push(value.clone());
            }
            continue;
        }
        count += 1;
        if count > MAX_UNIT_VALUES {
            return Err(Error::WorkingAllowance(
                "calculation exceeds the unit graph capacity",
            ));
        }
        pending.extend(node.inputs().iter().cloned());
    }
    Ok((count > 1).then_some(boundaries))
}

pub(super) fn compile_unit(
    calculation: &Calculation,
    output: CellRegion,
    budget: Budget,
) -> Result<WorkUnit, Error> {
    if let Some(alias) = calculation.value() {
        let expanded = super::unit::calculation(alias, output, budget)?;
        if expanded
            .value()
            .is_some_and(|value| value.same_capture(alias))
        {
            return Err(Error::InvalidParameter(
                "operation requires a declared executable calculation",
            ));
        }
        return expanded.compile(output, budget);
    }
    if let Some(boundaries) = constituents(calculation)? {
        return Ok(WorkUnit {
            inputs: boundaries.clone(),
            domain: calculation.domain(),
            output: calculation.primitive().unwrap().output_region(output),
            work: UnitWork::Gpu(compile_gpu_graph(calculation.clone(), &boundaries)?),
        });
    }
    calculation.plan_unit(output, budget)
}

pub(super) fn sampler(node: &Calculation) -> Option<&compiler::Primitive> {
    let primitive = node.primitive()?;
    (primitive.can_sample()
        && node
            .inputs()
            .iter()
            .all(|input| input.shape() == Shape::Pixel && input.value().is_some()))
    .then_some(primitive)
}

pub(super) fn is_copy(node: &Calculation) -> bool {
    node.primitive().is_some_and(|node| {
        matches!(
            node.operation.body.as_ref(),
            Body::Concatenate | Body::Argument { .. }
        )
    })
}

pub(super) fn parameters<T: std::any::Any>(key: &RetainedKey) -> Option<&T> {
    key.get::<Arguments>()?.parameters.get::<T>()
}

pub(super) fn pixel_kernel(calculation: &Calculation) -> bool {
    calculation.primitive().is_some_and(|primitive| {
        matches!(primitive.operation.body.as_ref(), Body::Gpu(_))
            && primitive.shape() == Shape::Pixel
    })
}

pub(super) fn image_resource(
    fragment: &mut Fragment,
    node: &Value,
    region: CellRegion,
    discover: bool,
) -> PlannedImage {
    let image = fragment.image(node.domain(), region);
    if discover && let Some(key) = node.identity() {
        fragment.resources[image.resource.0] = Storage::Image {
            key: key.clone(),
            pixels: None,
            layout: ImageLayout {
                domain: node.domain(),
                region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            },
        };
    }
    image
}

impl Value {
    /// The group declares its result; constituent scheduling is not an output identity.
    pub(super) fn declare_output(mut self, output: Output) -> Self {
        self.demand = Some(output);
        self
    }

    /// Restrict a read without changing calculation identity or materializing pixels.
    pub fn read(mut self, region: CellRegion) -> Result<Value, Error> {
        if !self.coverage().contains_region(region.validate()?) {
            return Err(Error::InvalidParameter("value read exceeds its coverage"));
        }
        let mut output = self.output(region)?;
        output.region = region;
        self.demand = Some(output);
        Ok(self)
    }

    /// Flatten and concatenate pixel regions by byte movement, not pixel arithmetic.
    #[cfg(test)]
    pub(crate) fn concatenate(mut inputs: Vec<Value>) -> Result<Value, Error> {
        if inputs.len() == 1
            && inputs[0].domain().meaning == Meaning::Arguments
            && inputs[0].coverage().x == 0
            && inputs[0].coverage().y == 0
            && inputs[0].coverage().height == 1
        {
            return Ok(inputs.pop().unwrap());
        }
        let mut cells = 0u32;
        for input in &inputs {
            let r = input.coverage();
            if input.shape() != Shape::Pixel {
                return Err(Error::InvalidParameter(
                    "concatenation requires pixel arguments",
                ));
            }
            cells = cells
                .checked_add(
                    r.width
                        .checked_mul(r.height)
                        .ok_or(Error::InvalidParameter(
                            "concatenated arguments exceed GPU addressing",
                        ))?,
                )
                .ok_or(Error::InvalidParameter(
                    "concatenated arguments exceed GPU addressing",
                ))?;
        }
        let region = CellRegion::new([0, 0], [cells, 1]);
        let regions = inputs.iter().map(Value::coverage).collect::<Vec<_>>();
        Value::new(
            Domain {
                bounds: Some(region),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::Arguments,
            },
            region,
            inputs,
            Operation {
                key: Some(RetainedKey::new(Arguments {
                    contract: "concatenated pixel regions v1",
                    parameters: RetainedKey::new(regions),
                })),
                body: Arc::new(Body::Concatenate),
            },
        )
    }

    pub(crate) fn shape(&self) -> Shape {
        (self.operation.as_ref() as &dyn std::any::Any)
            .downcast_ref::<applied::Applied>()
            .map_or(Shape::Pixel, |operation| operation.shape())
    }

    pub fn new(
        domain: Domain,
        coverage: CellRegion,
        inputs: Vec<Value>,
        operation: Operation,
    ) -> Result<Self, Error> {
        let operation = applied::Applied::capture(domain, coverage, &inputs, operation)?;
        let mut value = Self::captured(domain, inputs, Arc::new(operation));
        value.demand = Some(value.output(coverage)?);
        Ok(value)
    }

    #[cfg(test)]
    pub(crate) fn map_cpu_kernel(
        &self,
        inputs: Vec<Value>,
        wrap: impl FnOnce(Arc<dyn CpuKernel>) -> Arc<dyn CpuKernel>,
    ) -> Option<Value> {
        let applied =
            (self.operation.as_ref() as &dyn std::any::Any).downcast_ref::<applied::Applied>()?;
        let Body::Cpu(kernel) = applied.operation.body.as_ref() else {
            return None;
        };
        let mut decorated = applied.clone();
        decorated.operation.key = None;
        decorated.operation.body = Arc::new(Body::Cpu(wrap(kernel.clone())));
        Some(Self::captured(self.domain(), inputs, Arc::new(decorated)))
    }

    #[cfg(test)]
    pub(crate) fn described_bindings(&self) -> Option<Vec<(u32, FunctionBinding)>> {
        let region = self.coverage();
        let calculation = self
            .lower(
                region,
                Budget {
                    tile: [region.width, region.height],
                    scratch_bytes: 64 << 20,
                },
            )
            .ok()?;
        let primitive = calculation.primitive()?;
        match primitive.operation.body.as_ref() {
            Body::Gpu(body) => Some(body.bindings.clone()),
            _ => None,
        }
    }

    pub fn identity(&self) -> Option<&RetainedKey> {
        self.demand
            .as_ref()
            .map_or(&self.definition.content, |output| &output.content)
            .identity
            .as_ref()
    }

    pub fn coverage(&self) -> CellRegion {
        self.demand
            .as_ref()
            .expect("calculation input has requested coverage")
            .region
    }

    pub fn domain(&self) -> Domain {
        self.definition.domain
    }
}
