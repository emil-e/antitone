//! Original-sample photographic processing; CPU work is geometry, encoding and file safety.
use crate::{
    Error,
    export::{self, ExrPrecision, ImageSize, Overwrite, ProcessingMetadata},
    frame_raster::FrameRaster,
    geometry::{Composition, SourceRegion},
    gpu::{
        GpuFlatField, GpuPhotoGrade, ProcessingKernel, ProcessingSource, SourceGeometry,
        SubmissionGate,
    },
    local_adjustments::PhotoGradeSnapshot,
    processing::{Calibration, RollSettings},
    renderer::GpuRenderer,
    rendering::OutputTarget,
    scan::Scan,
};
use glam::{DMat3, DVec3};
use std::{collections::BTreeMap, path::Path, sync::Arc};
use wgpu::util::DeviceExt;

#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_edge_tests.rs"]
mod edge_tests;
#[cfg(all(test, feature = "editor", target_os = "macos"))]
#[path = "gpu_export_resource_tests.rs"]
mod resource_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_tests.rs"]
mod tests;

// Matching square source/destination units avoid repeated thin-strip work after quarter turns.
const TILE: [u32; 2] = export::BLOCK;
fn tile_size(area: SourceRegion) -> [u32; 2] {
    [TILE[0].min(area.width), TILE[1].min(area.height)]
}

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Tiff(OutputTarget),
    Exr(ExrPrecision),
}

/// Accepted state only. The scan leases the authoritative immutable development and correction.
pub struct Request<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub calibration: Calibration,
    pub roll: RollSettings,
    pub grade: PhotoGradeSnapshot,
    pub composition: Composition,
    pub format: Format,
}

/// Runs on a worker. There is exactly one submitted tile/readback at a time, with no CPU fallback.
pub fn write(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    (queue, submissions): (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    path: &Path,
    overwrite: Overwrite,
) -> Result<(), Error> {
    write_checked(
        renderer,
        device,
        (queue, submissions),
        request,
        path,
        overwrite,
        || Ok(()),
    )
}

// A bounded-unit stop hook lets resource diagnostics abort safely without changing export policy.
fn write_checked(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    (queue, submissions): (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    path: &Path,
    overwrite: Overwrite,
    mut check: impl FnMut() -> Result<(), Error>,
) -> Result<(), Error> {
    check()?;
    let Request {
        scan,
        field,
        calibration,
        roll,
        grade,
        composition,
        format,
    } = request;
    if composition.area() != scan.area {
        return Err(Error::InvalidParameter(
            "export composition belongs to another source area",
        ));
    }
    if scan.flat_field() != field.as_ref().map(|f| f.field()) {
        return Err(Error::Gpu("CPU and GPU flat-field resources differ".into()));
    }
    let metadata = ProcessingMetadata::with_grade(roll.response, &grade);
    let grade = GpuPhotoGrade::new(
        device,
        scan.processing_calibration(calibration)?,
        roll,
        grade,
        scan.camera_to_ap1,
        scan.area,
    )?;
    let mut tiles = OriginalTiles::new(
        renderer,
        (device, queue, submissions),
        (scan.camera_to_ap1, scan.area),
        field,
        grade,
        format,
    )?;
    let [width, height] = composition.size();
    let size = ImageSize { width, height };
    let tile_size = tiles.size;
    let mut read = |origin: [u32; 2]| {
        check()?;
        let pixels = scan.tile_region(
            scan.area,
            origin[0] - scan.area.x,
            origin[1] - scan.area.y,
            tile_size[0],
            tile_size[1],
        )?;
        let pixels = tiles.read(renderer, device, queue, submissions, origin, &pixels)?;
        check()?;
        Ok(pixels)
    };
    match format {
        Format::Tiff(target) => {
            export::tiff_encoded(path, size, target, overwrite, metadata, |origin, extent| {
                let linear = reconstruct(composition, origin, extent, &mut read)?;
                target.encode_u16(&linear.iter().map(|v| v.as_vec3()).collect::<Vec<_>>())
            })
        }
        Format::Exr(precision) => export::exr_blocks(
            path,
            size,
            precision,
            overwrite,
            metadata,
            |origin, extent| reconstruct(composition, origin, extent, &mut read),
        ),
    }
}

/// Preserve the scalar coverage weights and iteration order; no f32 preview geometry is used.
fn reconstruct(
    composition: Composition,
    origin: [u32; 2],
    extent: [u32; 2],
    mut read: impl FnMut([u32; 2]) -> Result<Vec<[f64; 4]>, Error>,
) -> Result<Vec<DVec3>, Error> {
    let area = composition.area();
    let size = tile_size(area);
    let raster = FrameRaster::full(composition)?;
    let mut samples = Vec::new();
    let mut groups = Vec::new();
    let mut tiles: BTreeMap<[u32; 2], Vec<(usize, usize)>> = BTreeMap::new();
    for y in origin[1]..origin[1] + extent[1] {
        for x in origin[0]..origin[0] + extent[0] {
            let start = samples.len();
            let mut add = |point: [u32; 2], weight| {
                let tile = std::array::from_fn(|i| {
                    let base = [area.x, area.y][i];
                    let end = base + [area.width, area.height][i];
                    (base + (point[i] - base) / size[i] * size[i]).min(end - size[i])
                });
                let index = ((point[1] - tile[1]) * size[0] + point[0] - tile[0]) as usize;
                tiles.entry(tile).or_default().push((samples.len(), index));
                samples.push((DVec3::ZERO, weight));
            };
            if composition.is_exact() {
                add(composition.source_pixel([x, y])?, 1.);
            } else {
                for (point, weight) in raster.footprint([x, y])?.pixels(area)? {
                    add(point, weight);
                }
            }
            groups.push(start..samples.len());
        }
    }
    for (origin, entries) in tiles {
        let pixels = read(origin)?;
        if pixels.len() != (size[0] * size[1]) as usize {
            return Err(Error::InvalidParameter(
                "invalid original export tile length",
            ));
        }
        for (sample, index) in entries {
            let p = pixels[index];
            samples[sample].0 = DVec3::new(p[0], p[1], p[2]);
        }
    }
    groups
        .into_iter()
        .map(|range| crate::sampling::weighted_mean_iter(samples[range].iter().copied().map(Ok)))
        .collect()
}

struct OriginalTiles {
    source: ProcessingSource,
    size: [u32; 2],
    geometry: bool,
    readback_pitch: u32,
    grade: GpuPhotoGrade,
    working: wgpu::Buffer,
    precise: Option<ProcessingKernel>,
    rendered: Option<(wgpu::Texture, wgpu::BindGroup, wgpu::RenderPipeline)>,
    readback: wgpu::Buffer,
}
impl OriginalTiles {
    fn new(
        renderer: &GpuRenderer,
        (device, queue, submissions): (&wgpu::Device, &wgpu::Queue, &SubmissionGate),
        (matrix, area): (DMat3, SourceRegion),
        field: Option<Arc<GpuFlatField>>,
        grade: GpuPhotoGrade,
        format: Format,
    ) -> Result<Self, Error> {
        let size = tile_size(area);
        let mut source =
            ProcessingSource::new(device, &vec![[0; 4]; (size[0] * size[1]) as usize], matrix)?;
        let geometry = field.is_some() || grade.snapshot().locals().iter().any(|a| a.is_active());
        if geometry {
            source.set_flat_field(
                field,
                SourceGeometry::new([area.x, area.y, size[0], size[1]], size)?,
            )?;
        }
        let readback_pitch = (size[0] * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let precise = if matches!(format, Format::Exr(_)) {
            Some(pollster::block_on(ProcessingKernel::new_export(
                device,
                queue,
                submissions.clone(),
            ))?)
        } else {
            None
        };
        let bytes =
            u64::from(size[0]) * u64::from(size[1]) * if precise.is_some() { 48 } else { 16 };
        let working = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("export original graded AP1 tile"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let rendered = if let Format::Tiff(target) = format {
            let resources = renderer.target(target);
            let dimensions = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("export tile dimensions"),
                contents: bytemuck::cast_slice(&[size[0], size[1], 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("export pinned ACES resources"),
                layout: &resources.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: working.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&resources.tables[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&resources.tables[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&resources.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: dimensions.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: resources.hues.as_entire_binding(),
                    },
                ],
            });
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("export original linear target tile"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            Some((texture, group, resources.pipeline.clone()))
        } else {
            None
        };
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("one export tile readback"),
            size: if rendered.is_some() {
                bytes + u64::from(readback_pitch) * u64::from(size[1])
            } else {
                bytes
            },
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Self {
            source,
            size,
            geometry,
            readback_pitch,
            grade,
            working,
            precise,
            rendered,
            readback,
        })
    }
    fn read(
        &mut self,
        renderer: &GpuRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        origin: [u32; 2],
        pixels: &[[u16; 4]],
    ) -> Result<Vec<[f64; 4]>, Error> {
        self.source.write(queue, pixels)?;
        if self.geometry {
            self.source.set_geometry(SourceGeometry::new(
                [origin[0], origin[1], self.size[0], self.size[1]],
                self.size,
            )?)?;
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        self.precise
            .as_ref()
            .unwrap_or(&renderer.kernel)
            .encode_grade(
                device,
                &mut encoder,
                &self.source,
                &self.working,
                &self.grade,
            )?;
        if let Some((texture, group, pipeline)) = &self.rendered {
            let view = texture.create_view(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("export per-original ACES, before reconstruction"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                // TIFF alone has target resources; EXR never executes the output transform.
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.draw(0..3, 0..1);
            }
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(self.readback_pitch),
                        rows_per_image: Some(self.size[1]),
                    },
                },
                texture.size(),
            );
            // Validate the photographic result too: ACES range limiting must not hide invalid AP1.
            encoder.copy_buffer_to_buffer(
                &self.working,
                0,
                &self.readback,
                u64::from(self.readback_pitch) * u64::from(self.size[1]),
                self.working.size(),
            );
        } else {
            encoder.copy_buffer_to_buffer(&self.working, 0, &self.readback, 0, self.working.size());
        }
        let submission = submissions.submit(queue, [encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.readback
            .map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = tx.send(result);
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|e| Error::Gpu(e.to_string()))?;
        rx.recv()
            .map_err(|e| Error::Gpu(e.to_string()))?
            .map_err(|e| Error::Gpu(e.to_string()))?;
        let mapped = self
            .readback
            .get_mapped_range(..)
            .map_err(|e| Error::Gpu(e.to_string()))?;
        let pixels = if self.precise.is_some() {
            bytemuck::cast_slice::<u8, [[f32; 4]; 3]>(&mapped)
                .iter()
                .map(|p| {
                    [
                        p[0][0] as f64 + p[1][0] as f64 + p[2][0] as f64,
                        p[0][1] as f64 + p[1][1] as f64 + p[2][1] as f64,
                        p[0][2] as f64 + p[1][2] as f64 + p[2][2] as f64,
                        p[0][3] as f64,
                    ]
                })
                .collect()
        } else {
            let split = (self.readback_pitch * self.size[1]) as usize;
            let working = bytemuck::cast_slice::<u8, [f32; 4]>(&mapped[split..]);
            let invalid = working
                .iter()
                .find_map(|p| p[..3].iter().position(|v| !v.is_finite()));
            if let Some(channel) = invalid {
                drop(mapped);
                self.readback.unmap();
                return Err(Error::NonFinitePixel { channel });
            }
            mapped[..split]
                .chunks_exact(self.readback_pitch as usize)
                .flat_map(|row| {
                    bytemuck::cast_slice::<u8, [f32; 4]>(&row[..self.size[0] as usize * 16])
                        .iter()
                        .map(|p| p.map(f64::from))
                })
                .collect()
        };
        drop(mapped);
        self.readback.unmap();
        Ok(pixels)
    }
}
