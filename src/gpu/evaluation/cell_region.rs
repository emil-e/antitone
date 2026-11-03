//! Integer cell addressing on an image's sampling lattice.

use crate::{Error, geometry::SourceRegion};

/// A half-open region on a sampling grid, which can extend before its origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRegion {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl CellRegion {
    pub fn new([x, y]: [i32; 2], [width, height]: [u32; 2]) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn packed(self) -> [u32; 4] {
        [self.x as u32, self.y as u32, self.width, self.height]
    }

    pub fn validate(self) -> Result<Self, Error> {
        if self.width == 0
            || self.height == 0
            || self.width > i32::MAX as u32
            || self.height > i32::MAX as u32
            || self.x.checked_add_unsigned(self.width).is_none()
            || self.y.checked_add_unsigned(self.height).is_none()
        {
            return Err(Error::InvalidParameter("invalid image cell region"));
        }
        Ok(self)
    }

    pub fn index(self, [x, y]: [i32; 2]) -> usize {
        debug_assert!(x >= self.x && x < self.right() && y >= self.y && y < self.bottom());
        (y - self.y) as usize * self.width as usize + (x - self.x) as usize
    }

    pub fn right(self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(self) -> i32 {
        self.y + self.height as i32
    }

    pub fn contains_region(self, region: Self) -> bool {
        self.validate().is_ok()
            && region.validate().is_ok()
            && region.x >= self.x
            && region.y >= self.y
            && region.right() <= self.right()
            && region.bottom() <= self.bottom()
    }

    pub fn expanded(self, support: [u32; 2], bounds: Option<Self>) -> Result<Self, Error> {
        self.validate()?;
        let mut low = [
            i64::from(self.x) - i64::from(support[0]),
            i64::from(self.y) - i64::from(support[1]),
        ];
        let mut high = [
            i64::from(self.right()) + i64::from(support[0]),
            i64::from(self.bottom()) + i64::from(support[1]),
        ];
        if let Some(bounds) = bounds {
            bounds.validate()?;
            for i in 0..2 {
                low[i] = low[i].max(i64::from([bounds.x, bounds.y][i]));
                high[i] = high[i].min(i64::from([bounds.right(), bounds.bottom()][i]));
            }
        }
        let error = || Error::InvalidParameter("image cell neighbourhood exceeds coordinate range");
        Self {
            x: low[0].try_into().map_err(|_| error())?,
            y: low[1].try_into().map_err(|_| error())?,
            width: (high[0] - low[0]).try_into().map_err(|_| error())?,
            height: (high[1] - low[1]).try_into().map_err(|_| error())?,
        }
        .validate()
    }

    pub fn source(self) -> Result<SourceRegion, Error> {
        self.validate()?;
        Ok(SourceRegion {
            x: self
                .x
                .try_into()
                .map_err(|_| Error::InvalidParameter("image region precedes source coordinates"))?,
            y: self
                .y
                .try_into()
                .map_err(|_| Error::InvalidParameter("image region precedes source coordinates"))?,
            width: self.width,
            height: self.height,
        })
    }
}

impl TryFrom<SourceRegion> for CellRegion {
    type Error = Error;

    fn try_from(region: SourceRegion) -> Result<Self, Error> {
        let limit = || Error::InvalidParameter("source exceeds image cell coordinates");
        Self {
            x: region.x.try_into().map_err(|_| limit())?,
            y: region.y.try_into().map_err(|_| limit())?,
            width: region.width,
            height: region.height,
        }
        .validate()
    }
}

impl TryFrom<[u32; 4]> for CellRegion {
    type Error = Error;

    fn try_from([x, y, width, height]: [u32; 4]) -> Result<Self, Error> {
        Self::try_from(SourceRegion {
            x,
            y,
            width,
            height,
        })
    }
}

impl From<[i32; 4]> for CellRegion {
    fn from([x, y, width, height]: [i32; 4]) -> Self {
        Self {
            x,
            y,
            width: width as u32,
            height: height as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_coordinates_are_checked_instead_of_reinterpreted_as_gpu_signed_bits() {
        assert_eq!(
            CellRegion::try_from([12u32, 7, 19, 11]).unwrap(),
            CellRegion::new([12, 7], [19, 11])
        );
        assert!(CellRegion::try_from([u32::MAX, 0, 1, 1]).is_err());
        assert!(CellRegion::try_from([i32::MAX as u32, 0, 1, 1]).is_err());
        assert!(CellRegion::try_from([0u32, 0, 0, 1]).is_err());
    }

    #[test]
    fn grid_padding_is_signed_but_native_source_reads_are_not() {
        let padded: CellRegion = [-7, -3, 19, 11].into();
        let inner: CellRegion = [0, 0, 12, 8].into();
        assert!(padded.validate().is_ok());
        assert!(padded.contains_region(inner));
        assert!(!inner.contains_region(padded));
        assert!(padded.source().is_err());
        assert_eq!(padded.packed(), [u32::MAX - 6, u32::MAX - 2, 19, 11]);
        assert_eq!(
            inner.expanded([7, 3], None).unwrap(),
            CellRegion::from([-7, -3, 26, 14])
        );
        assert_eq!(inner.expanded([7, 3], Some(inner)).unwrap(), inner);
        let rows = CellRegion::from([-4, -2, 4, 2]);
        assert_eq!(
            [
                rows.index([-4, -2]),
                rows.index([-1, -2]),
                rows.index([-4, -1]),
                rows.index([-1, -1])
            ],
            [0, 3, 4, 7]
        );
        let source: SourceRegion = [12, 7, 19, 11].into();
        assert_eq!(
            CellRegion::try_from(source).unwrap().source().unwrap(),
            source
        );
        assert!(
            CellRegion::try_from(SourceRegion {
                x: u32::MAX - 1,
                y: 0,
                width: 1,
                height: 1
            })
            .is_err()
        );
        assert!(
            CellRegion {
                x: i32::MAX,
                y: 0,
                width: 1,
                height: 1
            }
            .validate()
            .is_err()
        );
        assert!(CellRegion::from([0, 0, -1, 1]).validate().is_err());
    }
}
