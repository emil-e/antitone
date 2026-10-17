use super::*;
use crate::{
    geometry::SourcePoint,
    local_adjustments::{GradientMask, LocalAdjustment},
    processing::PhotoSettings,
};

// Diagnostic single-original GPU baseline, not a CPU renderer or production option.
pub(super) fn write_single(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    (queue, gate): (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    destination: (&Path, Overwrite),
    cached: bool,
    mut check: impl FnMut() -> Result<(), Error>,
) -> Result<(u64, u64, usize), Error> {
    check()?;
    let Request {
        scan,
        field,
        calibration,
        roll,
        grade,
        repairs,
        composition,
        format,
        look,
    } = request;
    if composition.area() != scan.area || scan.flat_field() != field.as_ref().map(|f| f.field()) {
        return Err(Error::InvalidParameter(
            "mismatched single-original diagnostic source",
        ));
    }
    let metadata = ProcessingMetadata::with_grade(roll.response, &grade).with_look(
        if matches!(format, Format::Tiff(_)) {
            look.as_ref().map(|asset| asset.lut().id().bytes())
        } else {
            None
        },
    );
    let repairs = crate::gpu::repairs::Snapshot::logical(repairs.to_vec())?;
    let mut tiles = timed!(
        Setup,
        OriginalTiles::new_with_look(
            renderer,
            (device, queue, gate),
            (scan.camera_to_ap1, scan.area),
            field,
            GpuPhotoGrade::new(
                device,
                scan.processing_calibration(calibration)?,
                roll,
                grade,
                scan.camera_to_ap1,
                scan.area
            )?,
            format,
            look.as_ref()
        )
    )?;
    tiles.set_repairs(device, repairs)?;
    let size = tiles.size;
    let mut reads = 0;
    let load = |origin: [u32; 2]| {
        reads += 1;
        let pixels = timed!(
            SourcePrepare,
            scan.tile_region(
                scan.area,
                origin[0] - scan.area.x,
                origin[1] - scan.area.y,
                size[0],
                size[1]
            )
        )?;
        tiles.read(renderer, device, queue, gate, origin, &pixels)
    };
    let mut reader = JobReader::new(load, check, cached);
    encode_output(composition, format, metadata, destination, &mut reader)?;
    let requests = reader.requests;
    let peak = reader.peak_bytes();
    drop(reader);
    Ok((requests, reads, peak))
}

fn words(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Vec<u32> {
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: buffer.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &readback, 0, buffer.size());
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    readback.map_async(wgpu::MapMode::Read, .., move |r| {
        tx.send(r).unwrap();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = readback.get_mapped_range(..).unwrap();
    let result = bytemuck::cast_slice::<u8, u32>(&mapped).to_vec();
    drop(mapped);
    readback.unmap();
    result
}

#[test]
fn paired_originals_preserve_pre_aces_words_flags_and_both_targets_at_coordinate_limits() {
    let (device, queue, renderer) = tests::context();
    let gate = SubmissionGate::default();
    let matrix = DMat3::from_cols(
        DVec3::new(0.6062, 0.0161, -0.0031),
        DVec3::new(0.3802, 1.1927, -0.1490),
        DVec3::new(0.0136, -0.2088, 1.1521),
    );
    let calibration = Calibration {
        dmin: DVec3::new(0.19, 0.14, 0.12),
        dmax: DVec3::new(0.008, 0.004, 0.002),
        input_floor: 1. / 65535.,
    };
    for area in [
        SourceRegion::from([10, 20, 1024, 768]),
        SourceRegion::from([(1 << 24) - 256, (1 << 24) - 128, 256, 128]),
    ] {
        let origins = [
            [area.x, area.y],
            [area.x + area.width - 128, area.y + area.height - 128],
        ];
        let geometry = SourceGeometry::original_pair(origins, TILE).unwrap();
        let input: Vec<_> = origins.iter().map(|&o| tests::tile(o, TILE)).collect();
        for corrected in [false, true] {
            for locals in [false, true] {
                for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                    let field = corrected.then(|| {
                        Arc::new(
                            crate::flat_field::FlatField::from_nodes(
                                [area.x, area.y, area.width, area.height],
                                [2, 2],
                                vec![
                                    DVec3::new(0.8, 0.9, 1.),
                                    DVec3::new(1.2, 1.1, 1.05),
                                    DVec3::new(1., 1.1, 0.9),
                                    DVec3::new(1.1, 0.95, 1.2),
                                ],
                            )
                            .unwrap(),
                        )
                    });
                    let calibration = field
                        .as_ref()
                        .map_or(Ok(calibration), |f| f.processing_calibration(calibration))
                        .unwrap();
                    let stages = if locals {
                        vec![
                            LocalAdjustment {
                                mask: GradientMask::Linear {
                                    start: SourcePoint([area.x as f64, area.y as f64]),
                                    end: SourcePoint([
                                        (area.x + area.width) as f64,
                                        (area.y + area.height) as f64,
                                    ]),
                                },
                                photo: PhotoSettings {
                                    exposure: 0.5,
                                    ..Default::default()
                                },
                                enabled: true,
                                invert: false,
                                strength: 0.7,
                            },
                            LocalAdjustment {
                                mask: GradientMask::Radial {
                                    center: SourcePoint([
                                        area.x as f64 + area.width as f64 / 2.,
                                        area.y as f64 + area.height as f64 / 2.,
                                    ]),
                                    radii: [area.width as f64 / 3., area.height as f64 / 3.],
                                    angle: 0.3,
                                    feather: 0.4,
                                },
                                photo: PhotoSettings {
                                    exposure: -0.25,
                                    saturation: 1.1,
                                    ..Default::default()
                                },
                                enabled: true,
                                invert: true,
                                strength: 0.7,
                            },
                        ]
                    } else {
                        vec![]
                    };
                    let snapshot = PhotoGradeSnapshot::new(
                        PhotoSettings {
                            exposure: if locals { 16. } else { 0. },
                            contrast: 0.8,
                            saturation: 1.1,
                            ..Default::default()
                        },
                        stages,
                    )
                    .unwrap();
                    let grade = GpuPhotoGrade::new(
                        &device,
                        calibration,
                        RollSettings::new_grade(),
                        snapshot,
                        matrix,
                        area,
                    )
                    .unwrap();
                    let field = field.map(|f| Arc::new(GpuFlatField::new(&device, f).unwrap()));
                    let mut single = OriginalTiles::new(
                        &renderer,
                        (&device, &queue, &gate),
                        (matrix, area),
                        field.clone(),
                        grade,
                        Format::Tiff(target),
                    )
                    .unwrap();
                    let mut expected = Vec::new();
                    let mut expected_words = Vec::new();
                    for (origin, input) in origins.iter().zip(&input) {
                        expected.extend(
                            single
                                .read(&renderer, &device, &queue, &gate, *origin, input)
                                .unwrap(),
                        );
                        expected_words.extend(words(&device, &queue, &single.working));
                    }
                    let mut pair = OriginalTiles::new_geometry(
                        &renderer,
                        (&device, &queue, &gate),
                        (matrix, geometry.size()),
                        field,
                        single.grade.clone(),
                        (Format::Tiff(target), None),
                        Some(geometry),
                    )
                    .unwrap();
                    let packed: Vec<_> = input.iter().flatten().copied().collect();
                    assert_eq!(
                        pair.read_geometry(
                            &renderer,
                            &device,
                            &queue,
                            &gate,
                            Some(geometry),
                            &packed
                        )
                        .unwrap(),
                        expected
                    );
                    assert_eq!(words(&device, &queue, &pair.working), expected_words);
                    let outside = SourceGeometry::original_pair(
                        [origins[0], [area.x + area.width, area.y]],
                        TILE,
                    );
                    if let Ok(outside) = outside {
                        assert!(
                            pair.read_geometry(
                                &renderer,
                                &device,
                                &queue,
                                &gate,
                                Some(outside),
                                &packed
                            )
                            .is_err(),
                            "second member must belong to the grade even without locals"
                        );
                    }
                }
            }
        }
    }
}
