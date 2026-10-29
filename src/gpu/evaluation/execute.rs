//! Materialize and execute declared work. No photographic-operation cases live here.
use super::*;
#[cfg(feature = "export")]
use crate::export_timing::timed;
use std::collections::HashMap;
use wgpu::util::DeviceExt;
#[cfg(not(feature = "export"))]
macro_rules! timed {
    ($stage:ident,$body:expr) => {
        $body
    };
}

#[derive(Default, Debug)]
pub struct Stats {
    pub submissions: usize,
    pub initializations: usize,
    pub source_cells: u64,
    pub buffer_allocations: usize,
    pub texture_allocations: usize,
    pub encoded_device_peak: u64,
    pub scratch_bytes: u64,
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
}
impl Allocation {
    fn bytes(&self) -> u64 {
        match self {
            Self::Input { .. } | Self::RenderTarget(..) => 0,
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
            ) => buffer.size() >= *bytes,
            (
                Self::Buffer {
                    buffer,
                    uniform: true,
                },
                Storage::Uniform(bytes),
            ) => buffer.size() >= bytes.len() as u64,
            (
                Self::Buffer {
                    buffer,
                    uniform: false,
                },
                Storage::Data(bytes),
            ) => buffer.size() >= bytes.len() as u64,
            (Self::Retained(value), Storage::Retained { key, bytes }) => {
                value.key == *key && value.pixels.size() == *bytes
            }
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
            Self::Texture(_, view) | Self::RenderTarget(_, view) => {
                wgpu::BindingResource::TextureView(view)
            }
        }
    }
}
#[derive(Default)]
pub struct Programs(std::sync::Mutex<HashMap<(Arc<str>, &'static str), wgpu::ComputePipeline>>);
#[cfg(all(test, feature = "editor"))]
impl Programs {
    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
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

#[derive(Default)]
pub struct Executor {
    kernels: Arc<Programs>,
    pool: Vec<Option<Allocation>>,
    retained: Vec<Arc<RetainedBuffer>>,
    retention: cache::Retention,
    #[cfg(all(test, target_os = "macos", feature = "export"))]
    pub timestamps: Option<(wgpu::QuerySet, u32)>,
}
#[cfg(all(test, feature = "editor"))]
pub struct ResultImage {
    pub pixels: wgpu::Buffer,
    pub stats: Stats,
}
impl Executor {
    #[cfg(all(test, feature = "editor"))]
    pub fn with_programs(kernels: Arc<Programs>) -> Self {
        Self::with_retention(kernels, Default::default())
    }
    pub(super) fn with_retention(kernels: Arc<Programs>, retention: cache::Retention) -> Self {
        Self {
            kernels,
            pool: Vec::new(),
            retained: Vec::new(),
            retention,
            #[cfg(all(test, target_os = "macos", feature = "export"))]
            timestamps: None,
        }
    }
    pub fn admit(limits: &wgpu::Limits, plan: &Plan) -> Result<(), Error> {
        if !plan.domain.bounds.contains_region(plan.region)
            || plan.peak_scratch_bytes > plan.scratch_budget
        {
            return Err(Error::InvalidParameter(
                "evaluation plan exceeds captured domain or budget",
            ));
        }
        let bytes = u64::from(plan.region.width) * u64::from(plan.region.height) * 16;
        if bytes > limits.max_buffer_size || bytes > limits.max_storage_buffer_binding_size {
            return Err(Error::InvalidParameter(
                "evaluation output exceeds GPU limits",
            ));
        }
        for tile in &plan.tiles {
            if tile.last_use[tile.result.resource.0] != tile.fragment.work.len() {
                return Err(Error::InvalidParameter(
                    "evaluation result lifetime ends before its sink",
                ));
            }
            for storage in &tile.slots {
                let invalid = match storage {
                    Storage::Buffer(bytes)
                    | Storage::Retained { bytes, .. }
                    | Storage::Input { bytes, .. } => {
                        *bytes > limits.max_buffer_size
                            || *bytes > limits.max_storage_buffer_binding_size
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
                        texture.format() != wgpu::TextureFormat::Rgba32Float
                            || texture.dimension() != wgpu::TextureDimension::D2
                            || texture.depth_or_array_layers() != 1
                            || texture.sample_count() != 1
                            || !texture
                                .usage()
                                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                            || [texture.width(), texture.height()]
                                .iter()
                                .any(|v| *v > limits.max_texture_dimension_2d)
                    }
                };
                if invalid {
                    return Err(Error::InvalidParameter(
                        "evaluation resource exceeds GPU limits",
                    ));
                }
            }
            for work in &tile.fragment.work {
                if let Work::Compute { groups, .. } = work
                    && groups
                        .iter()
                        .any(|v| *v == 0 || *v > limits.max_compute_workgroups_per_dimension)
                {
                    return Err(Error::InvalidParameter(
                        "evaluation dispatch exceeds GPU limits",
                    ));
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
            if let Work::Compute {
                code,
                entry,
                bindings,
                ..
            } = work
            {
                let key = (code.clone(), *entry);
                if self.kernels.0.lock().unwrap().contains_key(&key) {
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
                        visibility: wgpu::ShaderStages::COMPUTE,
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
                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                });
                if let Some(error) = pollster::block_on(scope.pop()) {
                    return Err(Error::Gpu(error.to_string()));
                }
                self.kernels
                    .0
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
        queue: &wgpu::Queue,
        gate: &crate::gpu::SubmissionGate,
        plan: &Plan,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<ResultImage>, Error> {
        if cancelled() {
            return Ok(None);
        }
        Self::admit(&device.limits(), plan)?;
        let pixels = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("evaluation retained result"),
            size: u64::from(plan.region.width) * u64::from(plan.region.height) * 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let Some(mut stats) = self.run_sink(
            device,
            queue,
            gate,
            plan,
            cancelled,
            |encoder, image, buffer| {
                for row in 0..image.region.height {
                    let destination = u64::from(image.region.y - plan.region.y + row)
                        * u64::from(plan.region.width)
                        + u64::from(image.region.x - plan.region.x);
                    encoder.copy_buffer_to_buffer(
                        buffer,
                        u64::from(row) * u64::from(image.region.width) * 16,
                        &pixels,
                        destination * 16,
                        u64::from(image.region.width) * 16,
                    );
                }
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        stats.buffer_allocations += 1;
        Ok(Some(ResultImage { pixels, stats }))
    }

    /// A bounded tile's GPU sink records into the same submission. It owns publication,
    /// reconstruction or readback, without requiring a retained full-source result buffer.
    pub fn run_sink(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gate: &crate::gpu::SubmissionGate,
        plan: &Plan,
        cancelled: impl Fn() -> bool,
        sink: impl FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
    ) -> Result<Option<Stats>, Error> {
        self.run_sink_with(
            device,
            queue,
            gate,
            plan,
            cancelled,
            Sink {
                encode: sink,
                submitted: || {},
            },
        )
    }

    pub fn run_sink_with<E, S>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gate: &crate::gpu::SubmissionGate,
        plan: &Plan,
        cancelled: impl Fn() -> bool,
        mut sink: Sink<E, S>,
    ) -> Result<Option<Stats>, Error>
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
        let mut stats = Stats::default();
        for tile in &plan.tiles {
            if cancelled() {
                return Ok(None);
            }
            self.materialize(
                device,
                tile,
                plan.scratch_budget,
                &mut stats,
                Materialization::Worker,
            )?;
            stats.scratch_bytes = stats.scratch_bytes.max(self.pool_bytes());
            for (id, storage) in tile.fragment.resources.iter().enumerate() {
                if let Storage::Uniform(bytes) | Storage::Data(bytes) = storage {
                    queue.write_buffer(
                        self.pool[tile.assignment[id]].as_ref().unwrap().buffer(),
                        0,
                        bytes,
                    );
                }
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            self.encode_tile(device, Some(queue), &mut encoder, tile, &mut stats, &[])?;
            let output = self.pool[tile.assignment[tile.result.resource.0]]
                .as_ref()
                .unwrap()
                .buffer();
            (sink.encode)(&mut encoder, tile.result, output)?;
            if cancelled() {
                return Ok(None);
            }
            #[cfg(any(test, feature = "ui"))]
            {
                stats.encoded_device_peak = stats
                    .encoded_device_peak
                    .max(crate::gpu::allocated_resource_bytes(device).unwrap_or(0) as u64);
            }
            stats.submissions += 1;
            submit(device, queue, gate, encoder, || {
                self.submitted();
                (sink.submitted)()
            })?;
        }
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(stats))
    }

    fn encode_tile(
        &self,
        device: &wgpu::Device,
        queue: Option<&wgpu::Queue>,
        encoder: &mut wgpu::CommandEncoder,
        tile: &Tile,
        stats: &mut Stats,
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
            match work {
                Work::Upload { source, output } => {
                    let queue = queue.ok_or(Error::InvalidParameter(
                        "record-only execution requires resident input",
                    ))?;
                    let values = timed!(SourcePrepare, source.read(output.region))?;
                    if values.len() != output.cells() as usize
                        || values.iter().flatten().any(|v| !v.is_finite())
                    {
                        return Err(Error::InvalidParameter(
                            "source differs from admitted evaluation layout",
                        ));
                    }
                    stats.source_cells += values.len() as u64;
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
                    if initialize.is_some_and(
                        |r| matches!(get(r),Allocation::Retained(value) if value.ready()),
                    ) {
                        continue;
                    }
                    if initialize.is_some() {
                        stats.initializations += 1;
                    }
                    let pipeline = self.kernels.0.lock().unwrap()[&(code.clone(), *entry)].clone();
                    let layout = pipeline.get_bind_group_layout(0);
                    let group = bind(device, &layout, bindings, &bound, inputs)?;
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
                    pipeline,
                    layout,
                    bindings,
                    target,
                } => {
                    let group = bind(device, layout, bindings, &bound, inputs)?;
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
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
        }

        Ok(())
    }
    pub fn resources(&self) -> Vec<Arc<RetainedBuffer>> {
        self.retained.clone()
    }
    pub fn submitted(&self) {
        for value in &self.retained {
            value.submitted();
        }
    }
    pub fn prepare(
        mut self,
        device: &wgpu::Device,
        plan: Plan,
        previous: Vec<Arc<RetainedBuffer>>,
    ) -> Result<Recorded, Error> {
        if plan.tiles.len() != 1 {
            return Err(Error::InvalidParameter(
                "record-only execution requires one bounded tile",
            ));
        }
        if plan.tiles[0]
            .fragment
            .work
            .iter()
            .any(|work| matches!(work, Work::Upload { .. }))
        {
            return Err(Error::InvalidParameter(
                "record-only execution requires resident input",
            ));
        }
        self.compile(device, &plan)?;
        let previous: Vec<_> = previous
            .into_iter()
            .filter(|value| value.retained_by(&self.retention))
            .collect();
        self.retained = previous.clone();
        self.materialize(
            device,
            &plan.tiles[0],
            plan.scratch_budget,
            &mut Stats::default(),
            Materialization::Capture,
        )?;
        let mut dormant = Vec::new();
        for value in previous {
            if plan.tiles[0].fragment.resources.iter().any(
                |storage| matches!(storage,Storage::Retained{key,..} if key.same_type(&value.key)),
            ) {
                continue;
            }
            dormant.push(value);
        }
        Ok(Recorded {
            executor: self,
            plan,
            dormant,
        })
    }
    fn materialize(
        &mut self,
        device: &wgpu::Device,
        tile: &Tile,
        budget: u64,
        stats: &mut Stats,
        mode: Materialization,
    ) -> Result<(), Error> {
        self.pool.truncate(tile.slots.len());
        let retained: u64 = tile
            .slots
            .iter()
            .enumerate()
            .map(|(index, storage)| {
                self.pool
                    .get(index)
                    .and_then(Option::as_ref)
                    .filter(|a| a.can_hold(storage))
                    .map_or_else(|| storage.bytes(), Allocation::bytes)
            })
            .sum();
        if retained > budget {
            self.pool.clear();
        }
        self.pool.resize_with(tile.slots.len(), || None);
        let previous = std::mem::take(&mut self.retained);
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
            // The preceding tile completed; drop its incompatible slot before replacement.
            self.pool[index] = None;
            if mode == Materialization::Capture
                && matches!(storage, Storage::Buffer(_) | Storage::Texture(_))
            {
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
                    stats.buffer_allocations += 1;
                    #[cfg(all(test, feature = "editor"))]
                    crate::gpu::test_support::evaluation_buffer_created(bytes);
                    Allocation::Buffer {
                        buffer: if mode != Materialization::Worker
                            && let Storage::Uniform(initial) | Storage::Data(initial) = storage
                        {
                            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("evaluation captured constants"),
                                contents: initial,
                                usage: if uniform {
                                    wgpu::BufferUsages::UNIFORM
                                } else {
                                    wgpu::BufferUsages::STORAGE
                                        | wgpu::BufferUsages::COPY_SRC
                                        | wgpu::BufferUsages::COPY_DST
                                },
                            })
                        } else {
                            device.create_buffer(&wgpu::BufferDescriptor {
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
                            })
                        },
                        uniform,
                    }
                }
                Storage::Retained { key, bytes } => {
                    let value = if let Some(value) = previous.iter().find(|value| {
                        value.key == *key
                            && value.pixels.size() == *bytes
                            && value.retained_by(&self.retention)
                    }) {
                        value.clone()
                    } else {
                        let lease = if mode == Materialization::Record {
                            None
                        } else {
                            self.retention.acquire(*bytes)
                        };
                        if mode == Materialization::Capture && lease.is_none() {
                            continue;
                        }
                        stats.buffer_allocations += 1;
                        #[cfg(all(test, feature = "editor"))]
                        crate::gpu::test_support::evaluation_buffer_created(*bytes);
                        Arc::new(RetainedBuffer::new(device, key.clone(), *bytes, lease))
                    };
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
                    stats.texture_allocations += 1;
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
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
                    });
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
    dormant: Vec<Arc<RetainedBuffer>>,
}
impl Recorded {
    #[cfg(test)]
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Buffer,
    ) -> Result<(), Error> {
        self.encode_into(device, encoder, input, input)
    }
    pub fn encode_into(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Buffer,
        destination: &wgpu::Buffer,
    ) -> Result<(), Error> {
        let tile = &self.plan.tiles[0];
        if matches!(
            tile.fragment.resources[tile.result.resource.0],
            Storage::Texture(_) | Storage::RenderTarget(_)
        ) {
            return Err(Error::InvalidParameter(
                "buffer sink requires raster storage",
            ));
        }
        if input == destination
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
        let mut execution = Executor::with_retention(
            self.executor.kernels.clone(),
            self.executor.retention.clone(),
        );
        execution.pool = self.executor.pool.clone();
        execution.retained = self.executor.retained.clone();
        let output_slot = tile.assignment[tile.result.resource.0];
        if input != destination
            && let Storage::Buffer(bytes) = tile.slots[output_slot]
            && bytes <= destination.size()
            && destination.usage().contains(wgpu::BufferUsages::STORAGE)
        {
            execution.pool[output_slot] = Some(Allocation::Input {
                slot: 1,
                bytes,
                writable: true,
            });
        }
        execution.materialize(
            device,
            tile,
            self.plan.scratch_budget,
            &mut Stats::default(),
            Materialization::Record,
        )?;
        let inputs = [input, destination];
        execution.encode_tile(device, None, encoder, tile, &mut Stats::default(), &inputs)?;
        let allocation = execution.pool[output_slot].as_ref().unwrap();
        let output = match allocation {
            Allocation::Input { slot, .. } => inputs[*slot],
            _ => allocation.buffer(),
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
        Ok(())
    }
    pub(crate) fn encode_work(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &[&wgpu::Buffer],
    ) -> Result<(), Error> {
        let mut execution = Executor::with_retention(
            self.executor.kernels.clone(),
            self.executor.retention.clone(),
        );
        execution.pool = self.executor.pool.clone();
        execution.retained = self.executor.retained.clone();
        let tile = &self.plan.tiles[0];
        execution.materialize(
            device,
            tile,
            self.plan.scratch_budget,
            &mut Stats::default(),
            Materialization::Record,
        )?;
        execution.encode_tile(device, None, encoder, tile, &mut Stats::default(), inputs)
    }
    pub fn resources(&self) -> Vec<Arc<RetainedBuffer>> {
        self.executor
            .resources()
            .into_iter()
            .chain(self.dormant.iter().cloned())
            .collect()
    }
    #[cfg(test)]
    pub fn cached<T: std::any::Any>(&self) -> Option<&Arc<RetainedBuffer>> {
        self.executor
            .retained
            .iter()
            .chain(&self.dormant)
            .find(|value| value.key.get::<T>().is_some())
    }
    pub fn submitted(&self) {
        self.executor.submitted();
    }
    pub fn byte_len(&self) -> u64 {
        self.executor.pool_bytes()
            + self
                .dormant
                .iter()
                .map(|value| value.pixels.size())
                .sum::<u64>()
    }
}
pub(super) fn submit(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gate: &crate::gpu::SubmissionGate,
    encoder: wgpu::CommandEncoder,
    submitted: impl FnOnce(),
) -> Result<(), Error> {
    let submission = timed!(Submit, gate.submit(queue, [encoder.finish()]));
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
        Binding::Read(_) | Binding::Input { .. } => {
            buffer(wgpu::BufferBindingType::Storage { read_only: true })
        }
        Binding::Write(_) => buffer(wgpu::BufferBindingType::Storage { read_only: false }),
        Binding::Uniform(_) => buffer(wgpu::BufferBindingType::Uniform),
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
        Binding::Texture(_) | Binding::Shared(Shared::Texture(_)) => wgpu::BindingType::Texture {
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
    inputs: &[&'a wgpu::Buffer],
) -> Result<wgpu::BindGroup, Error> {
    let entries = bindings
        .iter()
        .map(|(binding, resource)| {
            let resource = match resource {
                Binding::Shared(Shared::Buffer(buffer)) => buffer.as_entire_binding(),
                Binding::Shared(Shared::Texture(view) | Shared::StorageTexture(view)) => {
                    wgpu::BindingResource::TextureView(view)
                }
                Binding::Shared(Shared::Sampler(sampler)) => {
                    wgpu::BindingResource::Sampler(sampler)
                }
                Binding::Input { slot, bytes } => {
                    let buffer = inputs
                        .get(*slot)
                        .ok_or(Error::InvalidParameter("missing resident input binding"))?;
                    if buffer.size() < *bytes
                        || !buffer.usage().contains(wgpu::BufferUsages::STORAGE)
                    {
                        return Err(Error::InvalidParameter(
                            "resident input differs from declared storage",
                        ));
                    }
                    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer,
                        offset: 0,
                        size: std::num::NonZeroU64::new(*bytes),
                    })
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
pub fn read(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Vec<[f32; 4]> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("evaluation result readback"),
        size: buffer.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
    queue.submit([encoder.finish()]);
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
