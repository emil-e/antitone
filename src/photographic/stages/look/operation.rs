//! Immutable 3D-LUT resources and photographic Look operations.
use crate::{
    Error,
    looks::{LookId, Lut3D},
};
use std::sync::{Arc, Mutex};
use wgpu::util::DeviceExt;

#[cfg(feature = "render")]
use crate::gpu::evaluation::{
    CellRegion, Domain, Operation, Read, RetainedKey, Tiling, Value, value,
};

#[cfg(feature = "render")]
mod cache;

#[cfg(feature = "render")]
pub struct Looks {
    device: wgpu::Device,
    cache: Mutex<cache::Cache>,
    #[cfg(test)]
    uploads: std::sync::atomic::AtomicU64,
}

#[cfg(feature = "render")]
impl Looks {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            device: device.clone(),
            cache: Mutex::default(),
            #[cfg(test)]
            uploads: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn apply(&self, image: Value, chain: &crate::looks::LutChain) -> Result<Value, Error> {
        chain.assets().try_fold(image, |image, asset| {
            let look = self.resource(asset.lut())?;
            Ok(Value::from_operation(
                image.domain(),
                Some(image),
                LookOperation { look },
            ))
        })
    }

    #[cfg(test)]
    pub(crate) fn uploads(&self) -> u64 {
        self.uploads.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn live_resources(&self) -> usize {
        self.cache.lock().unwrap().live()
    }

    pub(crate) fn resource(&self, lut: &crate::looks::Lut3D) -> Result<Arc<Lut>, Error> {
        let (value, _uploaded) = self.cache.lock().unwrap().get(&self.device, lut)?;
        #[cfg(test)]
        if _uploaded {
            self.uploads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(value)
    }
}

#[cfg(feature = "render")]
pub(crate) fn applied_luts(image: &Value) -> Vec<Arc<Lut>> {
    // Output composition places looks on the unary path before the display endpoint.
    let mut luts = Vec::new();
    let mut node = Some(image);
    while let Some(image) = node {
        if let Some(operation) =
            (image.operation().as_ref() as &dyn std::any::Any).downcast_ref::<LookOperation>()
        {
            luts.push(operation.look.clone());
        }
        node = image.unary_input();
    }
    luts.reverse();
    luts
}

#[cfg(feature = "render")]
struct LookOperation {
    look: Arc<Lut>,
}

#[cfg(feature = "render")]
impl Operation for LookOperation {
    fn key(&self) -> Option<RetainedKey> {
        {
            #[derive(Debug, PartialEq, Hash)]
            struct Content(crate::looks::LookId);
            Some(RetainedKey::new(Content(self.look.id())))
        }
    }

    fn reads(&self, r: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, r)])
    }

    fn calculation(
        self: Arc<Self>,
        inputs: &[value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        let inputs = value::Calculation::references(inputs);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            self.look.calculation(domain.grid.coverage != 0),
        )
        .map(Some)
    }
}

#[cfg(test)]
mod test_dispatch;
#[cfg(test)]
pub(crate) use test_dispatch::Kernel;
#[cfg(all(test, target_os = "macos"))]
mod tests;

fn calculation(
    id: LookId,
    parameters: Parameters,
    nodes: crate::gpu::evaluation::FunctionBinding,
) -> crate::gpu::evaluation::value::Operation {
    use crate::gpu::evaluation::{Function, value};
    let code = [
        include_str!("arguments.wgsl"),
        include_str!("value.wgsl"),
        "fn pixel(cell:vec2i,position:vec2f,value:vec4f)->vec4f {return look_pixel(value);}",
    ]
    .join("\n");
    let captures = Function::new(code.clone(), "pixel").with_uniform(3, &parameters);
    let mut bindings = captures.bindings;
    bindings.push((2, nodes));
    value::Operation::new(
        "ACEScct/AP1 tetrahedral look v1",
        id,
        code,
        "pixel",
        value::Access::Pointwise,
    )
    .with_bindings(bindings)
}

fn prewarm_calculation(coverage: bool) -> crate::gpu::evaluation::value::Operation {
    let parameters = Parameters {
        minimum: [0.; 4],
        maximum: [1.; 4],
        reciprocal: [1.; 4],
        dimensions: [0, 2, 0, u32::from(coverage)],
    };
    calculation(
        LookId([0; 32]),
        parameters,
        crate::gpu::evaluation::FunctionBinding::Storage(
            bytemuck::cast_slice(&[[0f32; 4]; 8]).into(),
        ),
    )
}

#[cfg(feature = "render")]
pub(crate) fn prewarm_value(input: Value) -> Result<Value, Error> {
    let domain = input.domain();
    value::pointwise(input, prewarm_calculation(domain.grid.coverage != 0))
}

#[cfg(test)]
pub(crate) fn shader() -> String {
    [
        include_str!("arguments.wgsl"),
        include_str!("value.wgsl"),
        include_str!("dispatch.wgsl"),
    ]
    .join("\n")
}

pub struct Lut {
    buffer: wgpu::Buffer,
    minimum: [f32; 3],
    maximum: [f32; 3],
    size: u32,
    id: LookId,
}

impl Lut {
    /// Allocation-free admission for selection and candidate documents, before acceptance.
    pub fn require_limits(limits: &wgpu::Limits, look: &Lut3D) -> Result<(), Error> {
        let bytes = look.nodes().len() as u64 * 16;
        if bytes > limits.max_storage_buffer_binding_size || bytes > limits.max_buffer_size {
            return Err(Error::Gpu(
                "look exceeds this device's resource limits".into(),
            ));
        }
        Ok(())
    }

    pub fn new(device: &wgpu::Device, look: &Lut3D) -> Result<Arc<Self>, Error> {
        Self::require_limits(&device.limits(), look)?;
        let nodes = look
            .nodes()
            .iter()
            .map(|p| [p[0], p[1], p[2], 0.])
            .collect::<Vec<_>>();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable ACEScct/AP1 look samples"),
            contents: bytemuck::cast_slice(&nodes),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let (minimum, maximum) = look.domain();
        Ok(Arc::new(Self {
            buffer,
            minimum,
            maximum,
            size: look.size(),
            id: look.id(),
        }))
    }

    pub(crate) fn calculation(&self, coverage: bool) -> crate::gpu::evaluation::value::Operation {
        calculation(
            self.id,
            self.parameters(0, 0, coverage),
            crate::gpu::evaluation::FunctionBinding::Shared(
                crate::gpu::evaluation::Shared::Buffer(self.buffer.clone()),
            ),
        )
    }

    fn parameters(&self, count: u32, stride: u32, coverage: bool) -> Parameters {
        Parameters {
            minimum: [self.minimum[0], self.minimum[1], self.minimum[2], 0.],
            maximum: [self.maximum[0], self.maximum[1], self.maximum[2], 0.],
            reciprocal: std::array::from_fn(|i| {
                if i < 3 {
                    1. / (self.maximum[i] - self.minimum[i])
                } else {
                    0.
                }
            }),
            dimensions: [count, self.size, stride, u32::from(coverage)],
        }
    }
    pub fn id(&self) -> LookId {
        self.id
    }

    pub fn byte_len(&self) -> u64 {
        self.buffer.size()
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    minimum: [f32; 4],
    maximum: [f32; 4],
    reciprocal: [f32; 4],
    dimensions: [u32; 4],
}
