//! Orientation of developed rasters. LibRaw decodes and the DNG SDK develop sensor
//! data unrotated; this module maps between native sensor coordinates and the upright
//! developed coordinates every consumer outside RAW development sees. The transforms
//! replicate LibRaw's own flip handling (`LibRaw::flip_index`), so an upright development
//! is exactly what LibRaw's writer would emit with `user_flip` honoured.

use crate::{Error, geometry::SourceRegion as Rect};

/// Rotates a four-component u16 raster (LibRaw's developed RGB with a repeated green)
/// into developed orientation. The source stays untouched; the caller owns the result.
pub(crate) fn rotate_u16_rgba(
    source: &[u16],
    size: [u32; 2],
    rotation: Rotation,
) -> Result<(Vec<u16>, [u32; 2]), Error> {
    let [width, _] = size;
    let count = pixel_count(size)?;
    if source.len() != count * 4 {
        return Err(Error::Raw(
            "developed raster does not match its size".into(),
        ));
    }
    let mut rotated = Vec::new();
    rotated
        .try_reserve_exact(count * 4)
        .map_err(|_| Error::Raw("cannot allocate the rotated raster".into()))?;
    let developed = rotation.size(size);
    for y in 0..developed[1] {
        for x in 0..developed[0] {
            let [sx, sy] = rotation.point_developed_to_native([x, y], size);
            let offset = (sy as usize * width as usize + sx as usize) * 4;
            rotated.extend_from_slice(&source[offset..offset + 4]);
        }
    }
    Ok((rotated, developed))
}

/// Rotates a three-component f32 raster (the embedded-lens-corrected camera image)
/// into developed orientation, in place.
pub(crate) fn rotate_f32_rgb_in_place(
    pixels: &mut Vec<[f32; 3]>,
    size: &mut [u32; 2],
    rotation: Rotation,
) -> Result<(), Error> {
    let count = pixel_count(*size)?;
    if pixels.len() != count {
        return Err(Error::Raw(
            "corrected raster does not match its size".into(),
        ));
    }
    let developed = rotation.size(*size);
    let rotated = {
        let source = std::mem::take(pixels);
        let mut rotated = Vec::new();
        rotated
            .try_reserve_exact(source.len())
            .map_err(|_| Error::Raw("cannot allocate the rotated raster".into()))?;
        let [width, _] = *size;
        for y in 0..developed[1] {
            for x in 0..developed[0] {
                let [sx, sy] = rotation.point_developed_to_native([x, y], *size);
                rotated.push(source[sy as usize * width as usize + sx as usize]);
            }
        }
        rotated
    };
    *pixels = rotated;
    *size = developed;
    Ok(())
}

fn pixel_count(size: [u32; 2]) -> Result<usize, Error> {
    (size[0] as usize)
        .checked_mul(size[1] as usize)
        .filter(|&n| n > 0 && n <= isize::MAX as usize / 8)
        .ok_or_else(|| Error::Raw("invalid developed-raster size".into()))
}

/// Rotations cameras record for portrait and 180° shots. Mirrored and transverse
/// orientations do not occur for camera-held sources and are rejected at open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rotation {
    #[default]
    None,
    Cw180,
    Cw90,
    Ccw90,
}

impl Rotation {
    /// LibRaw's `sizes.flip`, dcraw's `"50132467"` orientation table: 0/3/5/6 are the
    /// unmirrored cases. A mirrored layout is a metadata defect for scanning, not a
    /// display preference this pipeline reinterprets.
    pub fn from_libraw_flip(flip: i32) -> Result<Self, Error> {
        match flip {
            0 => Ok(Self::None),
            3 => Ok(Self::Cw180),
            5 => Ok(Self::Ccw90),
            6 => Ok(Self::Cw90),
            _ => Err(Error::Raw(
                "mirrored or unknown RAW orientation is not supported".into(),
            )),
        }
    }

    /// TIFF orientation tags: 1/3/6/8 are the unmirrored cases.
    pub fn from_tiff_orientation(orientation: u32) -> Result<Self, Error> {
        match orientation {
            1 => Ok(Self::None),
            3 => Ok(Self::Cw180),
            6 => Ok(Self::Cw90),
            8 => Ok(Self::Ccw90),
            _ => Err(Error::Raw(
                "mirrored or unknown DNG orientation is not supported".into(),
            )),
        }
    }

    pub fn swaps(self) -> bool {
        matches!(self, Self::Cw90 | Self::Ccw90)
    }

    pub fn size(self, size: [u32; 2]) -> [u32; 2] {
        if self.swaps() {
            [size[1], size[0]]
        } else {
            size
        }
    }

    /// Native sensor pixel to the developed raster position, both as pixel indices.
    pub fn point_native_to_developed(self, point: [u32; 2], native_size: [u32; 2]) -> [u32; 2] {
        let [x, y] = point;
        let [width, height] = native_size;
        match self {
            Self::None => point,
            Self::Cw180 => [width - 1 - x, height - 1 - y],
            Self::Cw90 => [height - 1 - y, x],
            Self::Ccw90 => [y, width - 1 - x],
        }
    }

    /// Developed raster position to the native sensor pixel, both as pixel indices.
    pub fn point_developed_to_native(self, point: [u32; 2], native_size: [u32; 2]) -> [u32; 2] {
        let [x, y] = point;
        let [width, height] = native_size;
        match self {
            Self::None => point,
            Self::Cw180 => [width - 1 - x, height - 1 - y],
            Self::Cw90 => [y, height - 1 - x],
            Self::Ccw90 => [width - 1 - y, x],
        }
    }

    /// Maps a native sensor rectangle onto the developed raster rectangle covering the
    /// same pixels, against the native raster's full size.
    pub fn rect_native_to_developed(self, rect: Rect, native_size: [u32; 2]) -> Rect {
        if self == Self::None {
            return rect;
        }
        let [width, height] = native_size;
        match self {
            Self::None => rect,
            Self::Cw180 => Rect {
                x: width - rect.x - rect.width,
                y: height - rect.y - rect.height,
                width: rect.width,
                height: rect.height,
            },
            Self::Cw90 => Rect {
                x: height - rect.y - rect.height,
                y: rect.x,
                width: rect.height,
                height: rect.width,
            },
            Self::Ccw90 => Rect {
                x: rect.y,
                y: width - rect.x - rect.width,
                width: rect.height,
                height: rect.width,
            },
        }
    }

    /// Maps a developed-coordinate rectangle onto the native sensor rectangle covering
    /// the same pixels. Every rotation keeps rectangles axis-aligned.
    pub fn rect_developed_to_native(self, rect: Rect, native_size: [u32; 2]) -> Rect {
        if self == Self::None {
            return rect;
        }
        let [width, height] = native_size;
        match self {
            Self::None => rect,
            Self::Cw180 => Rect {
                x: width - rect.x - rect.width,
                y: height - rect.y - rect.height,
                width: rect.width,
                height: rect.height,
            },
            Self::Cw90 => Rect {
                x: rect.y,
                y: height - rect.x - rect.width,
                width: rect.height,
                height: rect.width,
            },
            Self::Ccw90 => Rect {
                x: width - rect.y - rect.height,
                y: rect.x,
                width: rect.height,
                height: rect.width,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libraw_flips_and_tiff_orientations_map_to_the_same_rotations() {
        // dcraw's "50132467" table: TIFF 8 -> flip 5, TIFF 6 -> flip 6.
        for (flip, orientation, rotation) in [
            (0, 1, Rotation::None),
            (3, 3, Rotation::Cw180),
            (6, 6, Rotation::Cw90),
            (5, 8, Rotation::Ccw90),
        ] {
            assert_eq!(Rotation::from_libraw_flip(flip).unwrap(), rotation);
            assert_eq!(
                Rotation::from_tiff_orientation(orientation).unwrap(),
                rotation
            );
        }
        for flip in [1, 2, 4, 7, -1, 9] {
            assert!(Rotation::from_libraw_flip(flip).is_err());
        }
        for orientation in [0, 2, 4, 5, 7, 9] {
            assert!(Rotation::from_tiff_orientation(orientation).is_err());
        }
    }

    #[test]
    fn rotations_are_their_own_inverse_on_points_and_swap_sizes() {
        let native = [40u32, 28u32];
        for rotation in [
            Rotation::None,
            Rotation::Cw180,
            Rotation::Cw90,
            Rotation::Ccw90,
        ] {
            assert_eq!(rotation.size(native).len(), 2);
            assert_eq!(rotation.swaps(), rotation.size(native) != native);
            for point in [[0u32, 0], [39, 27], [12, 7], [39, 0], [0, 27]] {
                let developed = rotation.point_native_to_developed(point, native);
                assert!(developed[0] < rotation.size(native)[0]);
                assert!(developed[1] < rotation.size(native)[1]);
                assert_eq!(rotation.point_developed_to_native(developed, native), point);
            }
        }
    }

    #[test]
    fn corner_pixels_move_where_the_display_rotation_puts_them() {
        let native = [40u32, 28u32];
        // Clockwise: the native top-left ends at the developed top-right.
        assert_eq!(
            Rotation::Cw90.point_native_to_developed([0, 0], native),
            [27, 0]
        );
        assert_eq!(
            Rotation::Cw90.point_developed_to_native([27, 0], native),
            [0, 0]
        );
        // Counter-clockwise: the native top-left ends at the developed bottom-left.
        assert_eq!(
            Rotation::Ccw90.point_native_to_developed([0, 0], native),
            [0, 39]
        );
        assert_eq!(
            Rotation::Cw180.point_native_to_developed([0, 0], native),
            [39, 27]
        );
    }

    #[test]
    fn developed_rectangles_map_to_covering_native_rectangles() {
        let native = [40u32, 28u32];
        let rect = Rect {
            x: 4,
            y: 6,
            width: 9,
            height: 5,
        };
        for rotation in [
            Rotation::None,
            Rotation::Cw180,
            Rotation::Cw90,
            Rotation::Ccw90,
        ] {
            let mapped = rotation.rect_developed_to_native(rect, native);
            assert_eq!(mapped.width * mapped.height, rect.width * rect.height);
            for [px, py] in [
                [rect.x, rect.y],
                [rect.x + rect.width - 1, rect.y],
                [rect.x, rect.y + rect.height - 1],
                [rect.x + rect.width - 1, rect.y + rect.height - 1],
                [rect.x + rect.width / 2, rect.y + rect.height / 2],
            ] {
                let [sx, sy] = rotation.point_developed_to_native([px, py], native);
                assert!(
                    sx >= mapped.x && sx < mapped.x + mapped.width,
                    "{rotation:?} maps {px},{py} outside {mapped:?}"
                );
                assert!(
                    sy >= mapped.y && sy < mapped.y + mapped.height,
                    "{rotation:?} maps {px},{py} outside {mapped:?}"
                );
            }
        }
        // A clockwise quarter turn moves the developed top edge to the native right edge.
        assert_eq!(
            Rotation::Cw90.rect_developed_to_native(rect, native),
            Rect {
                x: 6,
                y: 15,
                width: 5,
                height: 9
            }
        );
    }

    #[test]
    fn u16_rotation_relocates_pixels_and_preserves_components() {
        // 6 wide, 4 tall; each pixel carries four distinct component codes.
        let size = [6u32, 4];
        let source: Vec<u16> = (0..24u16)
            .flat_map(|i| [i, i + 100, i + 200, i + 300])
            .collect();
        let (rotated, developed) = rotate_u16_rgba(&source, size, Rotation::Cw90).unwrap();
        assert_eq!(developed, [4, 6]);
        // Clockwise: native top-left becomes the developed top-right pixel.
        assert_eq!(&rotated[3 * 4..3 * 4 + 4], &[0, 100, 200, 300]);
        // Native top-right (pixel 5) becomes the developed bottom-right.
        assert_eq!(
            &rotated[(5 * 4 + 3) * 4..(5 * 4 + 3) * 4 + 4],
            &[5, 105, 205, 305]
        );
        // Native bottom-left (pixel 18) becomes the developed top-left.
        assert_eq!(&rotated[0..4], &[18, 118, 218, 318]);
        let (rotated, developed) = rotate_u16_rgba(&source, size, Rotation::Cw180).unwrap();
        assert_eq!(developed, [6, 4]);
        assert_eq!(
            &rotated[(3 * 6 + 5) * 4..(3 * 6 + 5) * 4 + 4],
            &[0, 100, 200, 300]
        );
        assert!(rotate_u16_rgba(&source[..8], size, Rotation::Cw90).is_err());
    }

    #[test]
    fn f32_rotation_relocates_pixels_and_reports_swapped_size() {
        let mut size = [6u32, 4];
        let mut pixels: Vec<[f32; 3]> = (0..24).map(|i| [i as f32, 0., 0.]).collect();
        rotate_f32_rgb_in_place(&mut pixels, &mut size, Rotation::Ccw90).unwrap();
        assert_eq!(size, [4, 6]);
        // Counter-clockwise: native top-left becomes the developed bottom-left.
        assert_eq!(pixels[(5 * 4) as usize], [0., 0., 0.]);
        // Native top-right (pixel 5) becomes the developed top-left.
        assert_eq!(pixels[0], [5., 0., 0.]);
        let mut short: Vec<[f32; 3]> = vec![[0.; 3]; 3];
        let mut short_size = [4, 4];
        assert!(rotate_f32_rgb_in_place(&mut short, &mut short_size, Rotation::Cw90).is_err());
    }

    /// LibRaw's own writer transform (`LibRaw::flip_index` in file_write.cpp): for flip
    /// 3/5/6 the output pixel (row, col) must read the native pixel this module maps it
    /// to. Mirrored bits 1/2 alone are rejected at open, so only combinations 0/3/5/6
    /// are compared.
    #[test]
    fn developed_to_native_matches_libraw_flip_index() {
        let size = [52u32, 34u32];
        for flip in [0i32, 3, 5, 6] {
            let rotation = Rotation::from_libraw_flip(flip).unwrap();
            let [width, height] = size;
            for row in 0..rotation.size(size)[1] {
                for col in 0..rotation.size(size)[0] {
                    let mut source_row = row;
                    let mut source_col = col;
                    if flip & 4 != 0 {
                        std::mem::swap(&mut source_row, &mut source_col);
                    }
                    if flip & 2 != 0 {
                        source_row = height - 1 - source_row;
                    }
                    if flip & 1 != 0 {
                        source_col = width - 1 - source_col;
                    }
                    let expected = source_row * width + source_col;
                    let [sx, sy] = rotation.point_developed_to_native([col, row], size);
                    assert_eq!(sy * width + sx, expected, "flip {flip} at ({row}, {col})");
                }
            }
        }
    }
}
