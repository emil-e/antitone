use super::*;
use crate::{
    crop::Crop,
    frame_raster::FrameRaster,
    geometry::{Composition, FramePoint, PhotoOrientation, RollOrientation, SourceRegion},
};

struct Pattern(Domain);

fn pixel(x: u32, y: u32) -> [f32; 3] {
    [
        (x * 13 + y * 7) % 31,
        (x * 7 + y * 11) % 23,
        (x * 19 + y * 3) % 41,
    ]
    .map(|v| v as f32 / 11. - 0.6)
}

impl Source for Pattern {
    fn domain(&self) -> Domain {
        self.0
    }

    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("affine coverage test pattern"))
    }

    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        assert!(self.0.contains_region(region));
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right()).map(move |x| {
                    let [r, g, b] = pixel(x as u32, y as u32);
                    [r, g, b, 8.]
                })
            })
            .collect())
    }
}

#[test]
fn rotated_graph_footprints_match_scalar_coverage_across_output_blocks() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let area = SourceRegion {
        x: 120,
        y: 73,
        width: 23,
        height: 19,
    };
    let input = source(Arc::new(Pattern(Domain {
        bounds: Some(area.try_into().unwrap()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
    })));
    for angle in [-89_f64, -45., -13.7, 0., 19.3, 44., 89.] {
        let crop = Crop {
            center: [132.25, 82.75],
            size: [15, 9],
            angle: angle.to_radians(),
        }
        .constrained(area)
        .unwrap();
        let composition = Composition::with_crop(
            area,
            RollOrientation {
                horizontal: true,
                ..Default::default()
            },
            PhotoOrientation::default(),
            Some(crop),
        )
        .unwrap();
        for output in [composition.size(), [4, 3], [1, 1]] {
            let frame = FrameRaster::new(
                composition,
                FramePoint([0., 0.]),
                FramePoint(composition.size().map(f64::from)),
                output,
            )
            .unwrap();
            let region = Rect::new([0; 2], output);
            let grid = Grid {
                coverage: 8,
                ..Grid::from_frame(frame)
            };
            let root = reconstruct_complete(
                input.clone(),
                Domain {
                    bounds: Some(region),
                    grid,
                },
            )
            .unwrap();
            let mut actual = vec![[f32::NAN; 4]; (output[0] * output[1]) as usize];
            for block in tiled_units(region, [4, 3]).unwrap() {
                let request = context
                    .request(
                        &root,
                        Region {
                            bounds: block,
                            grid,
                        },
                        Tiling { tile: [4, 3] },
                    )
                    .unwrap()
                    .without_retention();
                let image = pollster::block_on(
                    context
                        .materialize(
                            &queue,
                            &context.materializations(),
                            request,
                            ImageFormat::Rgba32Float,
                            ImageBacking::Buffer,
                        )
                        .unwrap(),
                )
                .unwrap();
                let pixels =
                    crate::evaluation::read_buffer(&device, &queue, image.as_buffer().unwrap());
                for y in block.y..block.bottom() {
                    for x in block.x..block.right() {
                        actual[region.index([x, y])] = pixels[block.index([x, y])];
                    }
                }
            }
            for y in 0..output[1] {
                for x in 0..output[0] {
                    let coverage = frame.footprint([x, y]).unwrap();
                    let mut expected = [0.; 3];
                    for ([sx, sy], weight) in coverage.pixels(area).unwrap() {
                        for (dst, sample) in expected.iter_mut().zip(pixel(sx, sy)) {
                            *dst += f64::from(sample) * weight / coverage.area();
                        }
                    }
                    let actual = actual[(y * output[0] + x) as usize];
                    for c in 0..3 {
                        assert!(
                            (f64::from(actual[c]) - expected[c]).abs() < 2e-5,
                            "angle={angle} output={output:?} pixel={x},{y}: {actual:?} vs {expected:?}"
                        );
                    }
                    assert_eq!(actual[3], 8.);
                }
            }
        }
    }
}
