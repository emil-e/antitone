use super::*;
use antitone::{
    crop::Crop,
    geometry::{Flips, FramePoint, PhotoOrientation, QuarterTurns, RollOrientation, SourceRaster},
};

#[test]
fn composed_scope_bins_follow_frame_centres_not_the_source_bounding_rectangle() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let gpu = GpuScopes::new(&device).await.unwrap();
        let size = [17, 11];
        let area = SourceRegion {
            x: 12,
            y: 34,
            width: size[0],
            height: size[1],
        };
        let source = SourceRaster::new(area, size).unwrap();
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let mut pixels: Vec<_> = (0..size[0] * size[1])
                .map(|i| {
                    std::array::from_fn(|c| {
                        if c == 3 {
                            1.
                        } else {
                            target
                                .decode(
                                    ((i * (13 + c as u32 * 6)) % 256) as f64 / 256. + 0.5 / 256.,
                                )
                                .unwrap() as f32
                        }
                    })
                })
                .collect();
            pixels[0] = [f32::NAN, 0., 0., 1.];
            let image = texture(&device, &queue, size, &pixels);
            let crop = Crop::from_region(SourceRegion {
                x: 15,
                y: 37,
                width: 11,
                height: 5,
            });
            for turns in [
                QuarterTurns::Zero,
                QuarterTurns::One,
                QuarterTurns::Two,
                QuarterTurns::Three,
            ] {
                for reflected in [false, true] {
                    for crop in [
                        None,
                        Some(crop),
                        Some(
                            Crop {
                                angle: 13_f64.to_radians(),
                                ..crop
                            }
                            .constrained(area)
                            .unwrap(),
                        ),
                    ] {
                        let composition = Composition::with_crop(
                            area,
                            RollOrientation {
                                turns,
                                horizontal: reflected,
                                ..Default::default()
                            },
                            PhotoOrientation {
                                turns: QuarterTurns::One,
                                flips: Flips {
                                    vertical: reflected,
                                    ..Default::default()
                                },
                            },
                            crop,
                        )
                        .unwrap();
                        let extent = composition.size();
                        let output = if composition.is_exact() {
                            extent
                        } else {
                            [7, 5]
                        };
                        let frame = FrameRaster::new(
                            composition,
                            FramePoint([0., 0.]),
                            FramePoint(extent.map(f64::from)),
                            output,
                        )
                        .unwrap();
                        let mapped: Vec<_> = (0..output[0] * output[1])
                            .map(|i| {
                                let point = frame.source_point([
                                    f64::from(i % output[0]) + 0.5,
                                    f64::from(i / output[0]) + 0.5,
                                ]);
                                let xy = source.raster_point(point).0.map(|v| v.floor() as u32);
                                assert!(xy[0] < size[0] && xy[1] < size[1]);
                                pixels[(xy[1] * size[0] + xy[0]) as usize]
                            })
                            .collect();
                        let mapped_image = texture(&device, &queue, output, &mapped);
                        for plot in [Plot::Rgb, Plot::Luma, Plot::Vector] {
                            let settings = Settings {
                                target,
                                plot,
                                amplification: 1,
                            };
                            let (_, columns, counts) =
                                scopes::reference(output, &mapped, settings).unwrap();
                            let mut encoder = device.create_command_encoder(&Default::default());
                            gpu.encode_frame(&device, &mut encoder, &image, frame, settings)
                                .unwrap();
                            queue.submit([encoder.finish()]);
                            let actual = read(&device, &queue, &gpu.bins);
                            let totals = read(&device, &queue, &gpu.totals);
                            assert_eq!(
                                &totals[..3],
                                &[counts.valid, counts.invalid, counts.outside]
                            );
                            assert_eq!(&totals[4..], columns);
                            // Isolate geometry from neutral/primary chroma-bin edge rounding:
                            // compare with the same reduction on an independently mapped raster.
                            let mut encoder = device.create_command_encoder(&Default::default());
                            gpu.encode(&device, &mut encoder, &mapped_image, settings)
                                .unwrap();
                            queue.submit([encoder.finish()]);
                            let expected = read(&device, &queue, &gpu.bins);
                            assert!(actual == expected, "{target:?} {plot:?} {frame:?}");
                            assert_eq!(counts.valid + counts.invalid, output[0] * output[1]);
                        }
                    }
                }
            }
        }
    });
}
