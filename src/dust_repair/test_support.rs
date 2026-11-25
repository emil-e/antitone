use super::{Ap1, SourceRegion};

pub(crate) fn circle(size: [u32; 2], center: [i32; 2], radius: i32) -> Vec<bool> {
    let mut mask = vec![false; (size[0] * size[1]) as usize];
    for y in 0..size[1] as i32 {
        for x in 0..size[0] as i32 {
            mask[(y as u32 * size[0] + x as u32) as usize] =
                (x - center[0]).pow(2) + (y - center[1]).pow(2) <= radius.pow(2);
        }
    }
    mask
}

pub(crate) fn gradient(size: [u32; 2]) -> Vec<Ap1> {
    (0..size[1])
        .flat_map(|y| {
            (0..size[0]).map(move |x| {
                [
                    0.04 + x as f32 * 0.0001,
                    -0.02 + y as f32 * 0.0002,
                    1.1 + x as f32 * 0.0003,
                ]
            })
        })
        .collect()
}

pub(crate) fn outside_bits_equal(a: &[Ap1], b: &[Ap1], mask: &[bool]) -> bool {
    a.iter()
        .zip(b)
        .zip(mask)
        .all(|((a, b), hidden)| *hidden || a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits()))
}

pub(crate) fn active(size: [u32; 2]) -> SourceRegion {
    SourceRegion {
        x: 0,
        y: 0,
        width: size[0],
        height: size[1],
    }
}
