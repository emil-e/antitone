//! Materialize and execute declared work. No photographic-operation cases live here.
use crate::gpu::allocation::DeviceAllocationExt;

use super::*;
#[cfg(feature = "export")]
use crate::export_timing::timed;
use std::collections::HashMap;

#[cfg(not(feature = "export"))]
macro_rules! timed {
    ($stage:ident,$body:expr) => {
        $body
    };
}

/// Work counters for tests and opt-in performance diagnostics; empty in ordinary builds.
#[derive(Clone, Default, Debug)]
pub struct WorkCounts {
    #[cfg(test)]
    pub submissions: usize,
    #[cfg(test)]
    pub initializations: usize,
    #[cfg(test)]
    pub source_cells: u64,
    #[cfg(test)]
    pub buffer_allocations: usize,
    #[cfg(test)]
    pub texture_allocations: usize,
}

impl WorkCounts {
    pub(super) fn include(&mut self, _next: Self) {
        #[cfg(test)]
        {
            self.submissions += _next.submissions;
            self.initializations += _next.initializations;
            self.source_cells += _next.source_cells;
            self.buffer_allocations += _next.buffer_allocations;
            self.texture_allocations += _next.texture_allocations;
        }
    }

    pub(super) fn submitted(&mut self) {
        #[cfg(test)]
        {
            self.submissions += 1;
        }
    }

    pub(super) fn initialized(&mut self) {
        #[cfg(test)]
        {
            self.initializations += 1;
        }
    }

    pub(super) fn source_read(&mut self, _cells: u64) {
        #[cfg(test)]
        {
            self.source_cells += _cells;
        }
    }

    pub(super) fn buffers_allocated(&mut self, _count: usize) {
        #[cfg(test)]
        {
            self.buffer_allocations += _count;
        }
    }

    pub(super) fn texture_allocated(&mut self) {
        #[cfg(test)]
        {
            self.texture_allocations += 1;
        }
    }
}

#[derive(Clone)]
enum Allocation {
    Input {
        slot: usize,
        bytes: u64,
        writable: bool,
    },
    Buffer {
        buffer: wgpu::Buffer,
        uniform: bool,
    },
    Texture(wgpu::Texture, wgpu::TextureView),
    RenderTarget(wgpu::Texture, wgpu::TextureView),
    Retained(Arc<RetainedBuffer>),
    Resident(Arc<ImageValue>),
}

impl Allocation {
    fn bytes(&self) -> u64 {
        match self {
            Self::Input { .. } | Self::Resident(_) | Self::RenderTarget(..) => 0,
            Self::Buffer { buffer, .. } => buffer.size(),
            Self::Retained(value) => value.pixels.size(),
            Self::Texture(texture, _) => {
                u64::from(texture.width()) * u64::from(texture.height()) * 16
            }
        }
    }

    fn can_hold(&self, storage: &Storage) -> bool {
        match (self, storage) {
            (
                Self::Input {
                    slot: a,
                    bytes: b,
                    writable: c,
                },
                Storage::Input {
                    slot,
                    bytes,
                    writable,
                },
            ) => a == slot && b == bytes && c == writable,
            (
                Self::Input {
                    bytes: capacity,
                    writable: true,
                    ..
                },
                Storage::Buffer(bytes),
            ) => capacity >= bytes,
            (
                Self::Buffer {
                    buffer,
                    uniform: false,
                },
                Storage::Buffer(bytes),
            ) => buffer.size() == *bytes,
            (
                Self::Buffer {
                    buffer,
                    uniform: true,
                },
                Storage::Uniform(bytes),
            ) => buffer.size() == bytes.len() as u64,
            (
                Self::Buffer {
                    buffer,
                    uniform: false,
                },
                Storage::Data(bytes),
            ) => buffer.size() == bytes.len() as u64,
            (Self::Retained(value), Storage::Image { key, layout, .. }) => {
                value.key == *key
                    && value.descriptor.downcast_ref::<ImageLayout>() == Some(layout)
                    && value.pixels.size() == layout.bytes()
            }
            (Self::Resident(image), Storage::ResidentBuffer(next)) => Arc::ptr_eq(image, next),
            (Self::RenderTarget(texture, _), Storage::RenderTarget(next)) => texture == next,
            (Self::Texture(texture, _), Storage::Texture(size)) => {
                [texture.width(), texture.height()] == *size
            }
            _ => false,
        }
    }

    fn buffer(&self) -> &wgpu::Buffer {
        match self {
            Self::Buffer { buffer, .. } => buffer,
            Self::Retained(value) => &value.pixels,
            Self::Resident(image) => image.as_buffer().expect("validated resident buffer"),
            _ => unreachable!("validated resource class"),
        }
    }

    fn binding<'a>(&'a self, bytes: u64, inputs: &[&'a wgpu::Buffer]) -> wgpu::BindingResource<'a> {
        match self {
            Self::Input {
                slot,
                bytes: capacity,
                ..
            } => wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: inputs[*slot],
                offset: 0,
                size: std::num::NonZeroU64::new(if bytes == 0 { *capacity } else { bytes }),
            }),
            Self::Buffer { buffer, .. } => wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer,
                offset: 0,
                size: std::num::NonZeroU64::new(bytes),
            }),
            Self::Retained(value) => value.pixels.as_entire_binding(),
            Self::Resident(image) => image
                .as_buffer()
                .expect("validated resident buffer")
                .as_entire_binding(),
            Self::Texture(_, view) | Self::RenderTarget(_, view) => {
                wgpu::BindingResource::TextureView(view)
            }
        }
    }
}

#[derive(Clone)]
enum Program {
    Compute(wgpu::ComputePipeline),
    Render(wgpu::RenderPipeline),
}
#[derive(Default)]
pub struct Programs {
    compiled: std::sync::Mutex<HashMap<(Arc<str>, &'static str), Program>>,
    pub(super) libraries: sampling::Libraries,
}

#[cfg(all(test, feature = "editor"))]
impl Programs {
    pub fn len(&self) -> usize {
        self.compiled.lock().unwrap().len()
    }
}

pub(super) struct Encoded {
    pub counts: WorkCounts,
    pub resources: Allocations,
}

pub struct Sink<E, S> {
    pub encode: E,
    pub submitted: S,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Materialization {
    Worker,
    Capture,
    Record,
}

pub struct Executor {
    programs: Arc<Programs>,
    pool: Vec<Option<Allocation>>,
    retained: Vec<Arc<RetainedBuffer>>,
    used_images: Vec<Arc<ImageValue>>,
    copied_uploads: HashMap<usize, materialized::CopyCover>,
    scope: Materializations,
    pub(super) working: RunWorkingSet,
    #[cfg(all(test, target_os = "macos", feature = "export"))]
    pub timestamps: Option<(wgpu::QuerySet, u32)>,
}

#[cfg(all(test, feature = "editor"))]
pub struct ResultImage {
    pub pixels: wgpu::Buffer,
    pub counts: WorkCounts,
}

#[cfg(test)]
impl Default for Executor {
    fn default() -> Self {
        Self::with_retention(Arc::default(), Default::default())
    }
}

impl Executor {
    #[cfg(all(test, feature = "editor"))]
    pub fn with_programs(programs: Arc<Programs>) -> Self {
        Self::with_retention(programs, Default::default())
    }

    #[cfg(test)]
    pub(super) fn with_retention(programs: Arc<Programs>, retention: cache::Allowance) -> Self {
        let scope = Materializations::new(programs.clone(), retention);
        Self::with_scope(programs, scope)
    }

    pub(super) fn with_scope(programs: Arc<Programs>, scope: Materializations) -> Self {
        Self {
            programs,
            pool: Vec::new(),
            retained: Vec::new(),
            used_images: Vec::new(),
            copied_uploads: HashMap::new(),
            scope,
            working: RunWorkingSet::default(),
            #[cfg(all(test, target_os = "macos", feature = "export"))]
            timestamps: None,
        }
    }

    pub(super) fn with_working(mut self, working: RunWorkingSet) -> Self {
        self.working = working;
        self
    }

    pub fn admit(limits: &wgpu::Limits, plan: &Plan) -> Result<(), Error> {
        if !plan.domain.contains_region(plan.region) {
            return Err(Error::InvalidParameter(
                "evaluation plan exceeds captured domain",
            ));
        }
        let bytes = u64::from(plan.region.width) * u64::from(plan.region.height) * 16;
        if bytes > limits.max_buffer_size || bytes > limits.max_storage_buffer_binding_size {
            return Err(Error::EvaluationLimit(
                "evaluation output exceeds GPU limits",
            ));
        }
        for tile in &plan.tiles {
            if tile.fragment.resources.iter().any(|storage| matches!(storage, Storage::ResidentBuffer(image) if image.as_buffer().is_none())) {
                return Err(Error::InvalidParameter("resident GPU input requires buffer storage"));
            }
            if tile.last_use[tile.result.resource.0] != tile.fragment.work.len() {
                return Err(Error::InvalidParameter(
                    "evaluation result lifetime ends before its sink",
                ));
            }
            for storage in &tile.slots {
                if let Storage::Image { layout, pixels, .. } = storage {
                    if layout.format != ImageFormat::Rgba32Float
                        || layout.backing != ImageBacking::Buffer
                        || !layout.domain.contains_region(layout.region)
                    {
                        return Err(Error::InvalidParameter(
                            "generated image requires a complete float buffer layout",
                        ));
                    }
                    if let Some(image) = pixels {
                        super::materialized::validate(layout.domain, *layout, image)?;
                    }
                }
                let invalid = match storage {
                    Storage::Buffer(bytes) | Storage::Input { bytes, .. } => {
                        *bytes > limits.max_buffer_size
                            || *bytes > limits.max_storage_buffer_binding_size
                    }
                    Storage::Image { layout, .. } => {
                        layout.bytes() > limits.max_buffer_size
                            || layout.bytes() > limits.max_storage_buffer_binding_size
                    }
                    Storage::ResidentBuffer(image) => {
                        let buffer = image.as_buffer().ok_or(Error::InvalidParameter(
                            "resident GPU input requires buffer storage",
                        ))?;
                        if !buffer.usage().contains(wgpu::BufferUsages::STORAGE) {
                            return Err(Error::InvalidParameter(
                                "evaluation resource exceeds GPU limits",
                            ));
                        }
                        buffer.size() > limits.max_buffer_size
                            || buffer.size() > limits.max_storage_buffer_binding_size
                    }
                    Storage::Data(bytes) => {
                        bytes.len() as u64 > limits.max_storage_buffer_binding_size
                            || bytes.len() as u64 > limits.max_buffer_size
                    }
                    Storage::Uniform(bytes) => {
                        bytes.len() as u64 > limits.max_uniform_buffer_binding_size
                    }
                    Storage::Texture(size) => {
                        size.iter().any(|v| *v > limits.max_texture_dimension_2d)
                    }
                    Storage::RenderTarget(texture) => {
                        if texture.format() != wgpu::TextureFormat::Rgba32Float
                            || texture.dimension() != wgpu::TextureDimension::D2
                            || texture.depth_or_array_layers() != 1
                            || texture.sample_count() != 1
                            || !texture
                                .usage()
                                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                        {
                            return Err(Error::InvalidParameter(
                                "evaluation resource exceeds GPU limits",
                            ));
                        }
                        [texture.width(), texture.height()]
                            .iter()
                            .any(|v| *v > limits.max_texture_dimension_2d)
                    }
                };
                if invalid {
                    return Err(Error::EvaluationLimit(
                        "evaluation resource exceeds GPU limits",
                    ));
                }
            }
            for work in &tile.fragment.work {
                if let Work::Copy {
                    source,
                    target,
                    source_offset,
                    target_offset,
                    bytes,
                } = work
                {
                    let capacity = |resource: Resource| match &tile.fragment.resources[resource.0] {
                        Storage::Buffer(bytes) | Storage::Input { bytes, .. } => Some(*bytes),
                        Storage::Image { layout, .. } => Some(layout.bytes()),
                        Storage::ResidentBuffer(image) => image.as_buffer().map(wgpu::Buffer::size),
                        _ => None,
                    };
                    if *bytes == 0
                        || source_offset % 4 != 0
                        || target_offset % 4 != 0
                        || bytes % 4 != 0
                        || capacity(*source).is_none_or(|capacity| {
                            source_offset
                                .checked_add(*bytes)
                                .is_none_or(|end| end > capacity)
                        })
                        || capacity(*target).is_none_or(|capacity| {
                            target_offset
                                .checked_add(*bytes)
                                .is_none_or(|end| end > capacity)
                        })
                    {
                        return Err(Error::InvalidParameter(
                            "value copy exceeds its declared storage",
                        ));
                    }
                }
                if let Work::Compute { groups, .. } = work {
                    if groups.contains(&0) {
                        return Err(Error::InvalidParameter(
                            "evaluation dispatch exceeds GPU limits",
                        ));
                    }
                    if groups
                        .iter()
                        .any(|v| *v > limits.max_compute_workgroups_per_dimension)
                    {
                        return Err(Error::EvaluationLimit(
                            "evaluation dispatch exceeds GPU limits",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn pool_bytes(&self) -> u64 {
        self.pool.iter().flatten().map(Allocation::bytes).sum()
    }

    pub fn compile(&mut self, device: &wgpu::Device, plan: &Plan) -> Result<(), Error> {
        Self::admit(&device.limits(), plan)?;
        for tile in &plan.tiles {
            self.compile_work(device, &tile.fragment.work)?;
        }
        Ok(())
    }

    pub(crate) fn compile_work(
        &mut self,
        device: &wgpu::Device,
        work: &[Work],
    ) -> Result<(), Error> {
        for work in work {
            let bindings = match work {
                Work::Compute { bindings, .. } | Work::Render { bindings, .. } => Some(bindings),
                _ => None,
            };
            if bindings.is_some_and(|bindings| bindings.iter().any(|(_,binding)|
                matches!(binding, Binding::Shared(Shared::Image { image, .. }) if image.as_host().is_some()))) {
                return Err(Error::InvalidParameter("GPU bindings require resident storage"));
            }
            let (code, entry, bindings, render) = match work {
                Work::Compute {
                    code,
                    entry,
                    bindings,
                    ..
                } => (code, *entry, bindings, false),
                Work::Render {
                    program: RenderProgram::Shader(code),
                    bindings,
                    ..
                } => (code, "present_main", bindings, true),
                _ => continue,
            };
            {
                let key = (code.clone(), entry);
                if self.programs.compiled.lock().unwrap().contains_key(&key) {
                    continue;
                }

                let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("evaluation contracted GPU code"),
                    source: wgpu::ShaderSource::Wgsl(code.to_string().into()),
                });
                let entries = bindings
                    .iter()
                    .map(|(binding, resource)| wgpu::BindGroupLayoutEntry {
                        binding: *binding,
                        visibility: if render {
                            wgpu::ShaderStages::FRAGMENT
                        } else {
                            wgpu::ShaderStages::COMPUTE
                        },
                        count: None,
                        ty: binding_type(resource),
                    })
                    .collect::<Vec<_>>();
                let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("evaluation declared compute ABI"),
                    entries: &entries,
                });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("evaluation compute pipeline layout"),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                });
                let pipeline = if render {
                    Program::Render(device.create_render_pipeline(
                        &wgpu::RenderPipelineDescriptor {
                            label: Some("evaluation value attachment"),
                            layout: Some(&layout),
                            vertex: wgpu::VertexState {
                                module: &module,
                                entry_point: Some("main"),
                                compilation_options: Default::default(),
                                buffers: &[],
                            },
                            fragment: Some(wgpu::FragmentState {
                                module: &module,
                                entry_point: Some(entry),
                                compilation_options: Default::default(),
                                targets: &[Some(wgpu::ColorTargetState {
                                    format: wgpu::TextureFormat::Rgba32Float,
                                    blend: None,
                                    write_mask: wgpu::ColorWrites::ALL,
                                })],
                            }),
                            primitive: Default::default(),
                            depth_stencil: None,
                            multisample: Default::default(),
                            multiview_mask: None,
                            cache: None,
                        },
                    ))
                } else {
                    Program::Compute(device.create_compute_pipeline(
                        &wgpu::ComputePipelineDescriptor {
                            label: Some(entry),
                            layout: Some(&layout),
                            module: &module,
                            entry_point: Some(entry),
                            compilation_options: Default::default(),
                            cache: None,
                        },
                    ))
                };
                if let Some(error) = pollster::block_on(scope.pop()) {
                    return Err(Error::Gpu(error.to_string()));
                }

                self.programs
                    .compiled
                    .lock()
                    .unwrap()
                    .entry(key)
                    .or_insert(pipeline);
            }
        }
        Ok(())
    }

    #[cfg(all(test, feature = "editor"))]
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        plan: &Plan,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<ResultImage>, Error> {
        if cancelled() {
            return Ok(None);
        }
        Self::admit(&device.limits(), plan)?;
        let pixels = device.try_create_buffer(&wgpu::BufferDescriptor {
            label: Some("evaluation retained result"),
            size: u64::from(plan.region.width) * u64::from(plan.region.height) * 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })?;
        let Some(mut stats) =
            self.run_sink(device, queue, plan, cancelled, |encoder, image, buffer| {
                for row in 0..image.region.height {
                    let destination = (u64::from((image.region.y - plan.region.y) as u32)
                        + u64::from(row))
                        * u64::from(plan.region.width)
                        + u64::from((image.region.x - plan.region.x) as u32);
                    encoder.copy_buffer_to_buffer(
                        buffer,
                        u64::from(row) * u64::from(image.region.width) * 16,
                        &pixels,
                        destination * 16,
                        u64::from(image.region.width) * 16,
                    );
                }
                Ok(())
            })?
        else {
            return Ok(None);
        };
        stats.buffers_allocated(1);
        Ok(Some(ResultImage {
            pixels,
            counts: stats,
        }))
    }

    /// A bounded tile's GPU sink records into the same submission. It owns publication,
    /// reconstruction or readback, without requiring a retained full-source result buffer.
    #[cfg(test)]
    pub fn run_sink(
        &mut self,
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        plan: &Plan,
        cancelled: impl Fn() -> bool,
        sink: impl FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
    ) -> Result<Option<WorkCounts>, Error> {
        self.run_produced_sink(
            device,
            queue,
            plan,
            cancelled,
            (
                Sink {
                    encode: sink,
                    submitted: || {},
                },
                None,
            ),
            |_, _, _| {
                Err(Error::InvalidParameter(
                    "evaluation requires bounded worker production",
                ))
            },
        )
    }

    pub(in crate::gpu::evaluation) fn materializations(&self) -> Materializations {
        self.scope.clone()
    }

    pub(in crate::gpu::evaluation) fn run_produced_sink<E, S>(
        &mut self,
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        plan: &Plan,
        cancelled: impl Fn() -> bool,
        (mut sink, destination): (Sink<E, S>, Option<&wgpu::Buffer>),
        mut produce: impl FnMut(&value::Value, PlannedImage, &wgpu::Buffer) -> Result<WorkCounts, Error>,
    ) -> Result<Option<WorkCounts>, Error>
    where
        E: FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
        S: FnMut(),
    {
        if cancelled() {
            return Ok(None);
        }
        if plan.tiles.iter().any(|tile| {
            matches!(
                tile.fragment.resources[tile.result.resource.0],
                Storage::Texture(_) | Storage::RenderTarget(_)
            )
        }) {
            return Err(Error::InvalidParameter(
                "buffer sink requires raster storage",
            ));
        }

        timed!(Setup, self.compile(device, plan))?;
        let mut stats = WorkCounts::default();

        for tile in &plan.tiles {
            if cancelled() {
                return Ok(None);
            }

            self.materialize(device, tile, &mut stats, Materialization::Worker)?;
            for (id, storage) in tile.fragment.resources.iter().enumerate() {
                if let Storage::Uniform(bytes) | Storage::Data(bytes) = storage {
                    queue.write_buffer(
                        self.pool[tile.assignment[id]].as_ref().unwrap().buffer(),
                        0,
                        bytes,
                    );
                }
            }

            for work in &tile.fragment.work {
                if let Work::Produce { image, output } = work {
                    if cancelled() {
                        return Ok(None);
                    }
                    let pixels = self.pool[tile.assignment[output.resource.0]]
                        .as_ref()
                        .unwrap()
                        .buffer();
                    let nested = produce(image, *output, pixels)?;
                    stats.include(nested);
                }
            }
            if cancelled() {
                return Ok(None);
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            match self.encode_tile(
                device,
                (Some(queue), &cancelled),
                &mut encoder,
                tile,
                &mut stats,
                &[],
            ) {
                Err(Error::Cancelled) => return Ok(None),
                result => result?,
            }
            let output = self.pool[tile.assignment[tile.result.resource.0]]
                .as_ref()
                .unwrap()
                .buffer();
            (sink.encode)(&mut encoder, tile.result, output)?;
            if cancelled() {
                return Ok(None);
            }

            stats.submitted();
            let gpu_only = destination.is_some()
                && !tile.fragment.work.iter().any(|work| match work {
                    Work::Upload { output, .. } => {
                        !self.copied_uploads.contains_key(&output.resource.0)
                    }
                    Work::Produce { .. } => true,
                    _ => false,
                });
            if gpu_only {
                let mut resources = self.resources(&tile.fragment);
                resources.buffer(destination.unwrap());
                self.working
                    .submit_gpu(device, queue, encoder, resources, || {
                        self.submitted();
                        (sink.submitted)()
                    })?;
            } else {
                submit(device, queue, encoder, || {
                    self.submitted();
                    (sink.submitted)()
                })?;
            }
        }
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(stats))
    }

    fn encode_tile(
        &self,
        device: &wgpu::Device,
        (queue, cancelled): (Option<&crate::gpu::submission::Queue>, &dyn Fn() -> bool),
        encoder: &mut wgpu::CommandEncoder,
        tile: &Tile,
        stats: &mut WorkCounts,
        inputs: &[&wgpu::Buffer],
    ) -> Result<(), Error> {
        for allocation in self.pool.iter().flatten() {
            if let Allocation::Input { slot, bytes, .. } = allocation {
                let input = inputs
                    .get(*slot)
                    .ok_or(Error::InvalidParameter("missing resident input binding"))?;
                if input.size() < *bytes || !input.usage().contains(wgpu::BufferUsages::STORAGE) {
                    return Err(Error::InvalidParameter(
                        "resident input differs from declared storage",
                    ));
                }
            }
        }
        let get = |resource: Resource| self.pool[tile.assignment[resource.0]].as_ref().unwrap();
        let bound = |resource: Resource| {
            get(resource).binding(tile.fragment.resources[resource.0].bytes(), inputs)
        };
        #[cfg(all(test, target_os = "macos", feature = "export"))]
        let compute_range = (
            tile.fragment
                .work
                .iter()
                .position(|w| matches!(w, Work::Compute { .. })),
            tile.fragment
                .work
                .iter()
                .rposition(|w| matches!(w, Work::Compute { .. })),
        );
        let work = tile.fragment.work.iter();
        #[cfg(all(test, target_os = "macos", feature = "export"))]
        let work = work.enumerate();
        #[cfg(not(all(test, target_os = "macos", feature = "export")))]
        let work = work.map(|work| ((), work));
        for (_index, work) in work {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            match work {
                Work::Produce { .. } => {}
                Work::Copy {
                    source,
                    target,
                    source_offset,
                    target_offset,
                    bytes,
                } => {
                    let buffer = |resource| match get(resource) {
                        Allocation::Input { slot, .. } => inputs[*slot],
                        allocation => allocation.buffer(),
                    };
                    let source = buffer(*source);
                    let target = buffer(*target);
                    if source == target
                        || !source.usage().contains(wgpu::BufferUsages::COPY_SRC)
                        || !target.usage().contains(wgpu::BufferUsages::COPY_DST)
                        || source_offset
                            .checked_add(*bytes)
                            .is_none_or(|end| end > source.size())
                        || target_offset
                            .checked_add(*bytes)
                            .is_none_or(|end| end > target.size())
                    {
                        return Err(Error::InvalidParameter(
                            "value copy exceeds its buffer capabilities",
                        ));
                    }
                    encoder.copy_buffer_to_buffer(
                        source,
                        *source_offset,
                        target,
                        *target_offset,
                        *bytes,
                    );
                }
                Work::Upload { source, output } => {
                    let queue = queue.ok_or(Error::InvalidParameter(
                        "record-only execution requires resident input",
                    ))?;
                    if let Some(cover) = self.copied_uploads.get(&output.resource.0) {
                        cover.encode(encoder, get(output.resource).buffer())?;
                        continue;
                    }
                    let values = timed!(SourcePrepare, source.read(output.region))?;
                    if cancelled() {
                        return Err(Error::Cancelled);
                    }
                    if values.len() != output.cells() as usize
                        || !values.iter().fold(true, |valid, pixel| {
                            pixel
                                .iter()
                                .fold(valid, |valid, value| valid & value.is_finite())
                        })
                    {
                        return Err(Error::InvalidParameter(
                            "source differs from admitted evaluation layout",
                        ));
                    }
                    stats.source_read(values.len() as u64);
                    timed!(
                        SourceUpload,
                        queue.write_buffer(
                            get(output.resource).buffer(),
                            0,
                            bytemuck::cast_slice(&values),
                        )
                    );
                }
                Work::Compute {
                    code,
                    entry,
                    bindings,
                    groups,
                    initialize,
                } => {
                    if initialize.is_some_and(|r| {
                        matches!(
                            get(r),
                            Allocation::Retained(value) if value.ready()
                        )
                    }) {
                        continue;
                    }
                    if initialize.is_some() {
                        stats.initialized();
                    }

                    let Program::Compute(pipeline) =
                        self.programs.compiled.lock().unwrap()[&(code.clone(), *entry)].clone()
                    else {
                        unreachable!()
                    };
                    let layout = pipeline.get_bind_group_layout(0);
                    let group = bind(device, &layout, bindings, &bound)?;
                    let descriptor = wgpu::ComputePassDescriptor::default();
                    #[cfg(all(test, target_os = "macos", feature = "export"))]
                    let descriptor = wgpu::ComputePassDescriptor {
                        timestamp_writes: self
                            .timestamps
                            .as_ref()
                            .map(|(queries, base)| wgpu::ComputePassTimestampWrites {
                                query_set: queries,
                                beginning_of_pass_write_index: (compute_range.0 == Some(_index))
                                    .then_some(*base),
                                end_of_pass_write_index: (compute_range.1 == Some(_index))
                                    .then_some(*base + 1),
                            })
                            .filter(|v| {
                                v.beginning_of_pass_write_index.is_some()
                                    || v.end_of_pass_write_index.is_some()
                            }),
                        ..descriptor
                    };
                    let mut pass = encoder.begin_compute_pass(&descriptor);
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
                }
                Work::Render {
                    program,
                    bindings,
                    target,
                } => {
                    let (pipeline, layout) = match program {
                        RenderProgram::Captured { pipeline, layout } => {
                            (pipeline.clone(), layout.clone())
                        }
                        RenderProgram::Shader(code) => {
                            let Program::Render(pipeline) = self.programs.compiled.lock().unwrap()
                                [&(code.clone(), "present_main")]
                                .clone()
                            else {
                                unreachable!()
                            };
                            let layout = pipeline.get_bind_group_layout(0);
                            (pipeline, layout)
                        }
                    };
                    let group = bind(device, &layout, bindings, &bound)?;
                    let view = match get(*target) {
                        Allocation::Texture(_, view) | Allocation::RenderTarget(_, view) => view,
                        _ => unreachable!(),
                    };
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("evaluation contracted render pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
        }

        Ok(())
    }

    pub fn submitted(&self) {
        for value in &self.retained {
            value.submitted();
        }
        self.scope.submitted_values(
            self.used_images
                .iter()
                .chain(self.retained.iter().map(|value| value.image())),
        );
    }

    pub fn prepare(mut self, device: &wgpu::Device, plan: Plan) -> Result<Recorded, Error> {
        if plan.recording_passes() > Plan::MAX_RECORDING_PASSES {
            return Err(Error::EvaluationLimit(
                "recording exceeds the command-pass capacity",
            ));
        }

        if plan.tiles.len() != 1 {
            return Err(Error::InvalidParameter(
                "record-only execution requires one bounded tile",
            ));
        }
        if plan.tiles[0]
            .fragment
            .work
            .iter()
            .any(|work| matches!(work, Work::Upload { .. } | Work::Produce { .. }))
        {
            return Err(Error::InvalidParameter(
                "record-only execution requires resident input",
            ));
        }

        self.compile(device, &plan)?;
        let mut preparation = WorkCounts::default();
        self.materialize(
            device,
            &plan.tiles[0],
            &mut preparation,
            Materialization::Capture,
        )?;

        Ok(Recorded {
            executor: self,
            plan,
            preparation,
            relevance: None,
            #[cfg(feature = "editor")]
            prerequisites: Vec::new(),
        })
    }

    fn resources(&self, fragment: &Fragment) -> Allocations {
        let mut resources = Allocations::default();
        for allocation in self.pool.iter().flatten() {
            match allocation {
                Allocation::Buffer { buffer, .. } => resources.buffer(buffer),
                Allocation::Retained(value) => resources.image(value.image()),
                Allocation::Resident(image) => resources.image(image),
                Allocation::Texture(texture, _) => resources.texture(texture, allocation.bytes()),
                Allocation::RenderTarget(texture, _) => resources.texture(
                    texture,
                    u64::from(texture.width()) * u64::from(texture.height()) * 16,
                ),
                Allocation::Input { .. } => {}
            }
        }
        for image in &self.used_images {
            resources.image(image);
        }
        for work in &fragment.work {
            let bindings = match work {
                Work::Compute { bindings, .. } | Work::Render { bindings, .. } => bindings,
                _ => continue,
            };
            for (_, binding) in bindings {
                if let Binding::Shared(resource) = binding {
                    resources.shared(resource);
                }
            }
        }
        resources
    }

    fn materialize(
        &mut self,
        device: &wgpu::Device,
        tile: &Tile,
        stats: &mut WorkCounts,
        mode: Materialization,
    ) -> Result<(), Error> {
        self.pool.truncate(tile.slots.len());
        self.pool.resize_with(tile.slots.len(), || None);

        self.retained.clear();
        self.used_images.clear();
        self.copied_uploads.clear();
        if mode == Materialization::Worker {
            let mut commands = value::MAX_UNIT_VALUES;
            let mut remaining = value::MAX_UNIT_VALUES;
            for work in &tile.fragment.work {
                if let Work::Upload { source, output } = work
                    && let Some(cover) = materialized::CopyCover::find(
                        &self.scope,
                        &super::source(source.clone()),
                        output.region,
                        commands,
                        &mut remaining,
                    )?
                {
                    commands -= cover.commands;
                    self.used_images.extend(cover.images().cloned());
                    self.copied_uploads.insert(output.resource.0, cover);
                }
            }
        }
        for storage in &tile.slots {
            if let Storage::ResidentBuffer(image) = storage {
                self.used_images.push(image.clone());
            }
        }
        for work in &tile.fragment.work {
            if let Work::Compute { bindings, .. } | Work::Render { bindings, .. } = work {
                for (_, binding) in bindings {
                    if let Binding::Shared(Shared::Image { image, .. }) = binding {
                        self.used_images.push(image.clone());
                    }
                }
            }
        }
        for (index, storage) in tile.slots.iter().enumerate() {
            if self.pool[index]
                .as_ref()
                .is_some_and(|allocation| !allocation.can_hold(storage))
            {
                self.pool[index] = None;
            }
        }
        if mode == Materialization::Worker {
            self.working.admit_gpu(device)?;
        }
        for (index, storage) in tile.slots.iter().enumerate() {
            if self.pool[index]
                .as_ref()
                .is_some_and(|a| a.can_hold(storage))
            {
                if let Some(Allocation::Retained(value)) = &self.pool[index] {
                    self.retained.push(value.clone());
                }
                continue;
            }

            self.pool[index] = Some(match storage {
                Storage::Input {
                    slot,
                    bytes,
                    writable,
                } => Allocation::Input {
                    slot: *slot,
                    bytes: *bytes,
                    writable: *writable,
                },
                Storage::ResidentBuffer(image) => Allocation::Resident(image.clone()),
                Storage::Buffer(_) | Storage::Uniform(_) | Storage::Data(_) => {
                    let bytes = storage.bytes();
                    let uniform = matches!(storage, Storage::Uniform(_));
                    let limits = device.limits();
                    if bytes > limits.max_buffer_size
                        || bytes > limits.max_storage_buffer_binding_size
                        || (uniform && bytes > limits.max_uniform_buffer_binding_size)
                    {
                        return Err(Error::InvalidParameter(
                            "evaluation resource exceeds GPU limits",
                        ));
                    }

                    stats.buffers_allocated(1);
                    #[cfg(all(test, feature = "editor"))]
                    crate::gpu::test_support::evaluation_buffer_created(bytes);
                    Allocation::Buffer {
                        buffer: if mode != Materialization::Worker
                            && let Storage::Uniform(initial) | Storage::Data(initial) = storage
                        {
                            device.try_create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("evaluation captured constants"),
                                contents: initial,
                                usage: if uniform {
                                    wgpu::BufferUsages::UNIFORM
                                } else {
                                    wgpu::BufferUsages::STORAGE
                                        | wgpu::BufferUsages::COPY_SRC
                                        | wgpu::BufferUsages::COPY_DST
                                },
                            })?
                        } else {
                            device.try_create_buffer(&wgpu::BufferDescriptor {
                                label: Some("evaluation scheduled buffer slot"),
                                size: bytes,
                                usage: if uniform {
                                    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST
                                } else {
                                    wgpu::BufferUsages::STORAGE
                                        | wgpu::BufferUsages::COPY_SRC
                                        | wgpu::BufferUsages::COPY_DST
                                },
                                mapped_at_creation: false,
                            })?
                        },
                        uniform,
                    }
                }
                Storage::Image {
                    key,
                    layout,
                    pixels,
                } => {
                    let value = if let Some(image) = pixels {
                        Arc::new(RetainedBuffer::reused(
                            key.clone(),
                            Arc::new(*layout),
                            image.clone(),
                        ))
                    } else {
                        let Some(value) = self
                            .scope
                            .generated_image(device, key, *layout, true, true)?
                        else {
                            continue;
                        };
                        value
                    };
                    if !value.ready() {
                        stats.buffers_allocated(1);
                        #[cfg(all(test, feature = "editor"))]
                        crate::gpu::test_support::evaluation_buffer_created(layout.bytes());
                    }
                    self.retained.push(value.clone());
                    Allocation::Retained(value)
                }
                Storage::RenderTarget(texture) => Allocation::RenderTarget(
                    texture.clone(),
                    texture.create_view(&Default::default()),
                ),
                Storage::Texture(size) => {
                    if size
                        .iter()
                        .any(|v| *v > device.limits().max_texture_dimension_2d)
                    {
                        return Err(Error::InvalidParameter(
                            "evaluation texture exceeds GPU limits",
                        ));
                    }

                    stats.texture_allocated();
                    let texture = device.try_create_texture(&wgpu::TextureDescriptor {
                        label: Some("evaluation scheduled texture slot"),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba32Float,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })?;
                    let view = texture.create_view(&Default::default());
                    Allocation::Texture(texture, view)
                }
            });
        }
        Ok(())
    }
}

/// Immutable prepared work for callers that own submission. No source reads or waits occur here.
pub(crate) struct Recorded {
    executor: Executor,
    plan: Plan,
    pub(super) preparation: WorkCounts,
    relevance: Option<super::materializations::Relevance>,
    #[cfg(feature = "editor")]
    prerequisites: Vec<super::context::RecordedImage>,
}

impl Recorded {
    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn scheduled_bytes(&self) -> u64 {
        self.plan.scheduled_bytes()
    }

    #[cfg(feature = "editor")]
    pub(super) fn with_prerequisites(
        mut self,
        prerequisites: Vec<super::context::RecordedImage>,
    ) -> Self {
        self.prerequisites = prerequisites;
        self
    }

    pub(super) fn with_request(mut self, value: &value::Value, region: CellRegion) -> Self {
        self.relevance = Some(super::materializations::Relevance::new(value, region));
        self
    }

    #[cfg(test)]
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Buffer,
    ) -> Result<(), Error> {
        self.encode_into(device, encoder, input, input)
    }

    #[cfg(test)]
    pub fn encode_into(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Buffer,
        destination: &wgpu::Buffer,
    ) -> Result<(), Error> {
        self.encode_buffers(device, encoder, &[input], destination)
            .map(|_| ())
    }

    pub(crate) fn encode_result(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::Buffer,
    ) -> Result<(), Error> {
        self.encode_result_counts(device, encoder, destination)
            .map(|_| ())
    }

    pub(super) fn encode_result_counts(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::Buffer,
    ) -> Result<WorkCounts, Error> {
        self.encode_buffers(device, encoder, &[], destination)
            .map(|encoded| encoded.counts)
    }

    pub(super) fn encode_result_owned(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::Buffer,
    ) -> Result<Encoded, Error> {
        self.encode_buffers(device, encoder, &[], destination)
    }

    fn encode_buffers(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &[&wgpu::Buffer],
        destination: &wgpu::Buffer,
    ) -> Result<Encoded, Error> {
        let tile = &self.plan.tiles[0];
        if matches!(
            tile.fragment.resources[tile.result.resource.0],
            Storage::Texture(_) | Storage::RenderTarget(_)
        ) {
            return Err(Error::InvalidParameter(
                "buffer sink requires raster storage",
            ));
        }
        if inputs.first().is_some_and(|input| *input == destination)
            && tile.slots.iter().any(|slot| {
                matches!(
                    slot,
                    Storage::Input {
                        slot: 0,
                        writable: false,
                        ..
                    }
                )
            })
        {
            return Err(Error::InvalidParameter(
                "in-place delivery requires writable resident storage",
            ));
        }
        if !destination.usage().contains(wgpu::BufferUsages::COPY_DST)
            || destination.size() < u64::from(tile.result.cells()) * 16
        {
            return Err(Error::InvalidParameter(
                "recorded output differs from declared storage",
            ));
        }
        let mut execution =
            Executor::with_scope(self.executor.programs.clone(), self.executor.scope.clone())
                .with_working(self.executor.working.clone());
        execution.pool = self.executor.pool.clone();
        execution.retained = self.executor.retained.clone();
        let output_slot = tile.assignment[tile.result.resource.0];
        if inputs.iter().all(|input| *input != destination)
            && let Storage::Buffer(bytes) = tile.slots[output_slot]
            && bytes <= destination.size()
            && destination.usage().contains(wgpu::BufferUsages::STORAGE)
        {
            execution.pool[output_slot] = Some(Allocation::Input {
                slot: inputs.len(),
                bytes,
                writable: true,
            });
        }
        let mut stats = WorkCounts::default();
        execution.materialize(device, tile, &mut stats, Materialization::Record)?;
        let mut resources = Allocations::default();
        #[cfg(feature = "editor")]
        for prerequisite in &self.prerequisites {
            if let Some(commands) = &prerequisite.commands {
                let encoded =
                    commands.encode_result_owned(device, encoder, &prerequisite.pixels)?;
                stats.include(encoded.counts);
                resources.extend(&encoded.resources);
            }
            resources.buffered(&prerequisite.pixels);
        }
        let inputs: Vec<_> = inputs.iter().copied().chain([destination]).collect();
        execution.encode_tile(
            device,
            (None, &|| false),
            encoder,
            tile,
            &mut stats,
            &inputs,
        )?;
        let output = match execution.pool[output_slot].as_ref().unwrap() {
            Allocation::Input { slot, .. } => inputs[*slot],
            allocation => allocation.buffer(),
        };
        if output != destination {
            if !output.usage().contains(wgpu::BufferUsages::COPY_SRC) {
                return Err(Error::InvalidParameter(
                    "recorded result cannot be delivered",
                ));
            }
            encoder.copy_buffer_to_buffer(
                output,
                0,
                destination,
                0,
                u64::from(tile.result.cells()) * 16,
            );
        }
        resources.extend(&execution.resources(&tile.fragment));
        for input in inputs {
            resources.buffer(input);
        }
        Ok(Encoded {
            counts: stats,
            resources,
        })
    }

    pub(crate) fn encode_work(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &[&wgpu::Buffer],
    ) -> Result<WorkCounts, Error> {
        self.encode_work_owned(device, encoder, inputs)
            .map(|encoded| encoded.counts)
    }

    pub(super) fn encode_work_owned(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &[&wgpu::Buffer],
    ) -> Result<Encoded, Error> {
        let mut execution =
            Executor::with_scope(self.executor.programs.clone(), self.executor.scope.clone())
                .with_working(self.executor.working.clone());
        execution.pool = self.executor.pool.clone();
        execution.retained = self.executor.retained.clone();
        let tile = &self.plan.tiles[0];
        let mut stats = WorkCounts::default();
        execution.materialize(device, tile, &mut stats, Materialization::Record)?;
        execution.encode_tile(device, (None, &|| false), encoder, tile, &mut stats, inputs)?;
        let mut resources = execution.resources(&tile.fragment);
        for input in inputs {
            resources.buffer(input);
        }
        Ok(Encoded {
            counts: stats,
            resources,
        })
    }

    #[cfg(test)]
    pub fn cached_image(&self, meaning: Meaning) -> Option<&Arc<RetainedBuffer>> {
        self.executor.retained.iter().find(|value| {
            value
                .descriptor
                .downcast_ref::<ImageLayout>()
                .is_some_and(|layout| layout.domain.meaning == meaning)
        })
    }

    pub fn submitted(&self) {
        #[cfg(feature = "editor")]
        for prerequisite in &self.prerequisites {
            if let Some(commands) = &prerequisite.commands {
                commands.submitted();
            }
            prerequisite.pixels.submitted(&self.executor.scope);
        }
        if let Some(relevance) = &self.relevance {
            self.executor.scope.refresh(relevance);
        }
        self.executor.submitted();
    }

    pub(crate) fn reusable_inputs(&self) -> Vec<Arc<ImageValue>> {
        let mut images = Vec::new();
        for image in self
            .executor
            .used_images
            .iter()
            .chain(self.executor.retained.iter().map(|value| value.image()))
        {
            for input in std::iter::once(image).chain(image.reusable_inputs().iter()) {
                if !images.iter().any(|prior| Arc::ptr_eq(prior, input)) {
                    images.push(input.clone());
                }
            }
        }
        images
    }

    #[cfg(feature = "editor")]
    pub(crate) fn allocations(&self, allocations: &mut Allocations) {
        for value in self.executor.pool.iter().flatten() {
            match value {
                Allocation::Buffer { buffer, .. } => allocations.buffer(buffer),
                Allocation::Retained(image) => allocations.buffer(&image.pixels),
                Allocation::Resident(image) => allocations.image(image),
                Allocation::Texture(texture, _) => allocations.texture(texture, value.bytes()),
                Allocation::Input { .. } | Allocation::RenderTarget(..) => {}
            }
        }
        for image in &self.executor.used_images {
            allocations.image(image);
        }
        for prerequisite in &self.prerequisites {
            allocations.buffered(&prerequisite.pixels);
            if let Some(commands) = &prerequisite.commands {
                commands.allocations(allocations);
            }
        }
    }

    pub fn byte_len(&self) -> u64 {
        let bytes = self.executor.pool_bytes();
        #[cfg(feature = "editor")]
        let bytes = bytes
            + self
                .prerequisites
                .iter()
                .filter_map(|work| work.commands.as_ref())
                .map(Recorded::byte_len)
                .sum::<u64>();
        bytes
    }
}

pub(super) fn submit(
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    encoder: wgpu::CommandEncoder,
    submitted: impl FnOnce(),
) -> Result<(), Error> {
    let submission = timed!(Submit, queue.submit([encoder.finish()]));
    submitted();
    timed!(
        DeviceWait,
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10))
        })
    )
    .map_err(|e| Error::Gpu(e.to_string()))?;
    Ok(())
}

fn binding_type(resource: &Binding) -> wgpu::BindingType {
    let buffer = |ty| wgpu::BindingType::Buffer {
        ty,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    match resource {
        Binding::Read(_) => buffer(wgpu::BufferBindingType::Storage { read_only: true }),
        Binding::Write(_) => buffer(wgpu::BufferBindingType::Storage { read_only: false }),
        Binding::Uniform(_) => buffer(wgpu::BufferBindingType::Uniform),
        Binding::Shared(Shared::Image { view: None, .. }) => {
            buffer(wgpu::BufferBindingType::Storage { read_only: true })
        }
        Binding::Shared(Shared::Buffer(value)) => {
            buffer(if value.usage().contains(wgpu::BufferUsages::UNIFORM) {
                wgpu::BufferBindingType::Uniform
            } else {
                wgpu::BufferBindingType::Storage { read_only: true }
            })
        }
        Binding::Shared(Shared::StorageTexture(_)) => wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba32Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        Binding::Shared(Shared::Sampler(_)) => {
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering)
        }
        Binding::Shared(
            Shared::Texture(view)
            | Shared::Image {
                view: Some(view), ..
            },
        ) => wgpu::BindingType::Texture {
            sample_type: match view.texture().format().sample_type(None, None) {
                Some(wgpu::TextureSampleType::Uint) => wgpu::TextureSampleType::Uint,
                Some(wgpu::TextureSampleType::Sint) => wgpu::TextureSampleType::Sint,
                Some(wgpu::TextureSampleType::Depth) => wgpu::TextureSampleType::Depth,
                _ => wgpu::TextureSampleType::Float { filterable: false },
            },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        Binding::Texture(_) => wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    }
}

fn bind<'a>(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    bindings: &[(u32, Binding)],
    get: &impl Fn(Resource) -> wgpu::BindingResource<'a>,
) -> Result<wgpu::BindGroup, Error> {
    let entries = bindings
        .iter()
        .map(|(binding, resource)| {
            let resource = match resource {
                Binding::Shared(Shared::Image { image, view: None }) => image
                    .as_buffer()
                    .ok_or(Error::InvalidParameter(
                        "image binding is missing its texture view",
                    ))?
                    .as_entire_binding(),
                Binding::Shared(Shared::Buffer(buffer)) => buffer.as_entire_binding(),
                Binding::Shared(
                    Shared::Texture(view)
                    | Shared::StorageTexture(view)
                    | Shared::Image {
                        view: Some(view), ..
                    },
                ) => wgpu::BindingResource::TextureView(view),
                Binding::Shared(Shared::Sampler(sampler)) => {
                    wgpu::BindingResource::Sampler(sampler)
                }
                other => get(other.local().unwrap()),
            };
            Ok(wgpu::BindGroupEntry {
                binding: *binding,
                resource,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("evaluation declared bindings"),
        layout,
        entries: &entries,
    }))
}

#[cfg(all(test, feature = "editor"))]
pub fn read(
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    buffer: &wgpu::Buffer,
) -> Vec<[f32; 4]> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("evaluation result readback"),
        size: buffer.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
    queue.unobserved().submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |value| {
        send.send(value).unwrap()
    });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receive.recv().unwrap().unwrap();
    let result = bytemuck::cast_slice(&staging.get_mapped_range(..).unwrap()).to_vec();
    staging.unmap();
    result
}
