use super::*;
use crate::{
    frame_raster::FrameRaster,
    geometry::{Composition, FramePoint},
};

fn verify(frame: FrameRaster, exterior: bool) {
    let area = frame.composition.area();
    let input = Domain {
        bounds: Some(area.try_into().unwrap()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let mut grid = Grid::from_frame(frame);
    grid.coverage = 8;
    let output = Domain {
        bounds: Some(crate::gpu::evaluation::CellRegion::new(
            [0, 0],
            frame.output,
        )),
        grid,
        meaning: Meaning::Neutral,
    };
    let node = reconstruct(super::super::super::super::borrowed_input(input), output).unwrap();
    let op = (node.operation.as_ref() as &dyn std::any::Any)
        .downcast_ref::<Reconstruction>()
        .unwrap();
    let demand = op.input(output.finite().unwrap()).unwrap().unwrap();
    let mut units = Vec::new();
    for y in (demand.y..demand.bottom()).step_by(5) {
        for x in (demand.x..demand.right()).step_by(7) {
            let part: Rect = [x, y, 7.min(demand.right() - x), 5.min(demand.bottom() - y)].into();
            units.push((part, op.affected(part, output.finite().unwrap())));
        }
    }
    for y in 0..frame.output[1] {
        for x in 0..frame.output[0] {
            let needed: Vec<_> = if exterior {
                let low = frame.source_point([f64::from(x), f64::from(y)]).0;
                let high = frame.source_point([f64::from(x + 1), f64::from(y + 1)]).0;
                (low[1].floor() as i64..high[1].ceil() as i64)
                    .flat_map(|sy| {
                        (low[0].floor() as i64..high[0].ceil() as i64).map(move |sx| {
                            [
                                sx.clamp(i64::from(area.x), i64::from(area.right() - 1)) as u32,
                                sy.clamp(i64::from(area.y), i64::from(area.bottom() - 1)) as u32,
                            ]
                        })
                    })
                    .collect()
            } else {
                frame
                    .footprint([x, y])
                    .unwrap()
                    .pixels(area)
                    .unwrap()
                    .map(|(p, _)| p)
                    .collect()
            };
            assert!(!needed.is_empty());
            for point in needed {
                let (_, affected) = units
                    .iter()
                    .find(|(part, _)| contains(*part, point))
                    .expect("every needed original cell is demanded");
                assert!(
                    affected.is_some_and(|r| contains(r, [x, y])),
                    "source={point:?} destination=({x},{y})"
                );
            }
        }
    }
}
fn contains(region: Rect, p: [u32; 2]) -> bool {
    let [x, y] = p.map(|v| v as i32);
    x >= region.x && y >= region.y && x < region.right() && y < region.bottom()
}

#[test]
fn conservative_contribution_windows_cover_rotated_forward_footprints() {
    let composition = Composition::with_crop(
        [100, 80, 400, 300].into(),
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [300., 230.],
            size: [64, 48],
            angle: 0.31,
        }),
    )
    .unwrap();
    verify(
        FrameRaster::new(
            composition,
            FramePoint([15., 14.]),
            FramePoint([25., 20.]),
            [4, 3],
        )
        .unwrap(),
        false,
    );
}
#[test]
fn wholly_exterior_footprints_receive_the_actual_edge_cell() {
    let composition = Composition::new(
        [100, 80, 24, 18].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    verify(
        FrameRaster {
            composition,
            min: FramePoint([-8., -9.]),
            max: FramePoint([-4., -3.]),
            output: [4, 6],
        },
        true,
    );
}
