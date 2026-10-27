//! Spatial coordinates and mappings shared by sampling, composition and presentation.
use crate::{Error, coverage::Coverage, crop::Crop};

/// A half-open integer region in original developed-source coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl SourceRegion {
    pub fn validate(self) -> Result<Self, Error> {
        if self.width == 0
            || self.height == 0
            || self.x.checked_add(self.width).is_none()
            || self.y.checked_add(self.height).is_none()
        {
            return Err(Error::InvalidParameter("invalid source region"));
        }
        Ok(self)
    }

    /// Constrains an interactive selection to this region, including a one-pixel minimum.
    pub fn clamped_selection(self, start: SourcePoint, end: SourcePoint) -> Self {
        let x = (start.0[0].min(end.0[0]).floor() as u32).clamp(self.x, self.x + self.width - 1);
        let y = (start.0[1].min(end.0[1]).floor() as u32).clamp(self.y, self.y + self.height - 1);
        let right = (start.0[0].max(end.0[0]).ceil() as u32).clamp(x + 1, self.x + self.width);
        let bottom = (start.0[1].max(end.0[1]).ceil() as u32).clamp(y + 1, self.y + self.height);
        Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }

    pub fn contains_region(self, region: Self) -> bool {
        region.width > 0
            && region.height > 0
            && region.x >= self.x
            && region.y >= self.y
            && u64::from(region.x) + u64::from(region.width)
                <= u64::from(self.x) + u64::from(self.width)
            && u64::from(region.y) + u64::from(region.height)
                <= u64::from(self.y) + u64::from(self.height)
    }
}

impl From<[u32; 4]> for SourceRegion {
    fn from([x, y, width, height]: [u32; 4]) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// A continuous position in original developed-source pixel-edge units.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(transparent))]
pub struct SourcePoint(pub [f64; 2]);

/// A continuous position in the oriented full active area, before crop/straightening.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrientedPoint(pub [f64; 2]);

/// A continuous position in the final composed frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramePoint(pub [f64; 2]);

/// A continuous position in a declared raster grid, in pixel-edge units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterPoint(pub [f64; 2]);

/// Exact nearest-centre sampling of an original source region onto a raster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceRaster {
    region: SourceRegion,
    size: [u32; 2],
}

impl SourceRaster {
    pub fn new(region: SourceRegion, size: [u32; 2]) -> Result<Self, Error> {
        region.validate()?;
        if size.contains(&0) {
            return Err(Error::InvalidParameter("empty raster grid"));
        }
        Ok(Self { region, size })
    }

    pub fn region(self) -> SourceRegion {
        self.region
    }

    pub fn size(self) -> [u32; 2] {
        self.size
    }

    pub fn source_point(self, point: RasterPoint) -> SourcePoint {
        SourcePoint([
            f64::from(self.region.x)
                + point.0[0] * f64::from(self.region.width) / f64::from(self.size[0]),
            f64::from(self.region.y)
                + point.0[1] * f64::from(self.region.height) / f64::from(self.size[1]),
        ])
    }

    pub fn raster_point(self, point: SourcePoint) -> RasterPoint {
        RasterPoint([
            (point.0[0] - f64::from(self.region.x)) * f64::from(self.size[0])
                / f64::from(self.region.width),
            (point.0[1] - f64::from(self.region.y)) * f64::from(self.size[1])
                / f64::from(self.region.height),
        ])
    }

    /// Selects an original integer source pixel, without floating-point rounding.
    pub fn source_pixel(self, pixel: [u32; 2]) -> Result<[u32; 2], Error> {
        if pixel[0] >= self.size[0] || pixel[1] >= self.size[1] {
            return Err(Error::InvalidParameter("pixel outside raster grid"));
        }
        let origin = [self.region.x, self.region.y];
        let extent = [self.region.width, self.region.height];
        Ok(std::array::from_fn(|axis| {
            // Decompose the product so even full u32 dimensions cannot overflow u64.
            let product = u64::from(pixel[axis]) * u64::from(extent[axis]);
            let size = u64::from(self.size[axis]);
            origin[axis]
                + (product / size + (2 * (product % size) + u64::from(extent[axis])) / (2 * size))
                    as u32
        }))
    }
}

/// Independently editable acquisition or displayed-axis reflections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct Flips {
    pub horizontal: bool,
    pub vertical: bool,
}

impl Flips {
    pub fn is_identity(&self) -> bool {
        !self.horizontal && !self.vertical
    }

    fn apply(self, point: [f64; 2], size: [u32; 2]) -> [f64; 2] {
        [
            if self.horizontal {
                f64::from(size[0]) - point[0]
            } else {
                point[0]
            },
            if self.vertical {
                f64::from(size[1]) - point[1]
            } else {
                point[1]
            },
        ]
    }
}

/// Clockwise quarter-turn count; serialization rejects noncanonical values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(rename_all = "snake_case"))]
pub enum QuarterTurns {
    #[default]
    Zero,
    One,
    Two,
    Three,
}

impl QuarterTurns {
    fn is_zero(&self) -> bool {
        *self == Self::Zero
    }

    fn clockwise(self) -> Self {
        match self {
            Self::Zero => Self::One,
            Self::One => Self::Two,
            Self::Two => Self::Three,
            Self::Three => Self::Zero,
        }
    }

    fn counterclockwise(self) -> Self {
        match self {
            Self::Zero => Self::Three,
            Self::One => Self::Zero,
            Self::Two => Self::One,
            Self::Three => Self::Two,
        }
    }

    fn size(self, size: [u32; 2]) -> [u32; 2] {
        match self {
            Self::One | Self::Three => [size[1], size[0]],
            _ => size,
        }
    }

    fn apply(self, [x, y]: [f64; 2], [width, height]: [u32; 2]) -> [f64; 2] {
        match self {
            Self::Zero => [x, y],
            Self::One => [f64::from(height) - y, x],
            Self::Two => [f64::from(width) - x, f64::from(height) - y],
            Self::Three => [y, f64::from(width) - x],
        }
    }

    fn inverse(self) -> Self {
        match self {
            Self::One => Self::Three,
            Self::Three => Self::One,
            _ => self,
        }
    }
}

/// Shared scan orientation. Flat flip fields preserve the original schema-3 representation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct RollOrientation {
    #[cfg_attr(
        feature = "project",
        serde(default, skip_serializing_if = "QuarterTurns::is_zero")
    )]
    pub turns: QuarterTurns,
    pub horizontal: bool,
    pub vertical: bool,
}

impl RollOrientation {
    fn discrete(self) -> PhotoOrientation {
        PhotoOrientation {
            turns: self.turns,
            flips: Flips {
                horizontal: self.horizontal,
                vertical: self.vertical,
            },
        }
    }

    pub fn is_identity(&self) -> bool {
        self.discrete().is_identity()
    }

    /// Turns the shared scan baseline; per-photo exceptions are applied afterward.
    pub fn turn_clockwise(&mut self) {
        self.turns = self.turns.clockwise();
        std::mem::swap(&mut self.horizontal, &mut self.vertical);
    }

    pub fn turn_counterclockwise(&mut self) {
        self.turns = self.turns.counterclockwise();
        std::mem::swap(&mut self.horizontal, &mut self.vertical);
    }
}

impl From<Flips> for RollOrientation {
    fn from(flips: Flips) -> Self {
        Self {
            horizontal: flips.horizontal,
            vertical: flips.vertical,
            turns: QuarterTurns::Zero,
        }
    }
}

/// Per-photo quarter-turns followed by flips in the resulting displayed axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct PhotoOrientation {
    pub turns: QuarterTurns,
    pub flips: Flips,
}

impl PhotoOrientation {
    pub fn is_identity(&self) -> bool {
        self.turns.is_zero() && self.flips.is_identity()
    }

    fn apply(self, point: [f64; 2], size: [u32; 2]) -> [f64; 2] {
        self.flips
            .apply(self.turns.apply(point, size), self.turns.size(size))
    }

    fn unapply(self, point: [f64; 2], size: [u32; 2]) -> [f64; 2] {
        self.turns.inverse().apply(
            self.flips.apply(point, self.turns.size(size)),
            self.turns.size(size),
        )
    }

    /// Rotates the currently displayed photo, including its displayed-axis flips.
    pub fn turn_clockwise(&mut self) {
        self.turns = self.turns.clockwise();
        std::mem::swap(&mut self.flips.horizontal, &mut self.flips.vertical);
    }

    pub fn turn_counterclockwise(&mut self) {
        self.turns = self.turns.counterclockwise();
        std::mem::swap(&mut self.flips.horizontal, &mut self.flips.vertical);
    }
}

/// Full active-source orientation. Mapping is continuous and does not clamp coverage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Orientation {
    area: SourceRegion,
    roll: RollOrientation,
    photo: PhotoOrientation,
}

impl Orientation {
    pub fn new(
        area: SourceRegion,
        roll: RollOrientation,
        photo: PhotoOrientation,
    ) -> Result<Self, Error> {
        Ok(Self {
            area: area.validate()?,
            roll,
            photo,
        })
    }

    pub fn area(self) -> SourceRegion {
        self.area
    }

    pub fn size(self) -> [u32; 2] {
        self.size_for([self.area.width, self.area.height])
    }

    fn size_for(self, size: [u32; 2]) -> [u32; 2] {
        self.photo.turns.size(self.roll.turns.size(size))
    }

    pub fn source_to_oriented(self, point: SourcePoint) -> OrientedPoint {
        let local = [
            point.0[0] - f64::from(self.area.x),
            point.0[1] - f64::from(self.area.y),
        ];
        let size = [self.area.width, self.area.height];
        let rolled = self.roll.discrete().apply(local, size);
        OrientedPoint(self.photo.apply(rolled, self.roll.turns.size(size)))
    }

    pub fn oriented_to_source(self, point: OrientedPoint) -> SourcePoint {
        let size = [self.area.width, self.area.height];
        let unphoto = self.photo.unapply(point.0, self.roll.turns.size(size));
        let local = self.roll.discrete().unapply(unphoto, size);
        SourcePoint([
            local[0] + f64::from(self.area.x),
            local[1] + f64::from(self.area.y),
        ])
    }
}

/// Source-anchored frame followed by exact orientation; selected source content stays immutable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Composition {
    orientation: Orientation,
    crop: Crop,
}

impl Composition {
    pub fn new(
        area: SourceRegion,
        roll: RollOrientation,
        photo: PhotoOrientation,
    ) -> Result<Self, Error> {
        Self::with_crop(area, roll, photo, None)
    }

    pub fn with_crop(
        area: SourceRegion,
        roll: RollOrientation,
        photo: PhotoOrientation,
        crop: Option<Crop>,
    ) -> Result<Self, Error> {
        let orientation = Orientation::new(area, roll, photo)?;
        let crop = crop.unwrap_or(Crop::from_region(area));
        crop.region(area)?;
        Ok(Self { orientation, crop })
    }

    pub fn area(self) -> SourceRegion {
        self.orientation.area()
    }

    pub fn region(self) -> SourceRegion {
        self.crop
            .region(self.area())
            .expect("validated composition coverage")
    }

    pub fn size(self) -> [u32; 2] {
        self.orientation.size_for(self.crop.size)
    }

    pub fn crop(self) -> Crop {
        self.crop
    }

    pub fn is_exact(self) -> bool {
        self.crop.angle == 0.
    }

    /// Reflection changes the sign of a displayed image-under-frame straighten action.
    pub fn handedness(self) -> f64 {
        let roll = self.orientation.roll;
        let photo = self.orientation.photo.flips;
        if roll.horizontal ^ roll.vertical ^ photo.horizontal ^ photo.vertical {
            -1.
        } else {
            1.
        }
    }

    pub fn source_to_frame(self, point: SourcePoint) -> FramePoint {
        let local = self.crop.local_point(point);
        let rolled = self
            .orientation
            .roll
            .discrete()
            .apply(local, self.crop.size);
        FramePoint(
            self.orientation
                .photo
                .apply(rolled, self.orientation.roll.turns.size(self.crop.size)),
        )
    }

    pub fn frame_to_source(self, point: FramePoint) -> SourcePoint {
        let unphoto = self
            .orientation
            .photo
            .unapply(point.0, self.orientation.roll.turns.size(self.crop.size));
        self.crop.source_point(
            self.orientation
                .roll
                .discrete()
                .unapply(unphoto, self.crop.size),
        )
    }

    pub fn coverage(self, min: FramePoint, max: FramePoint) -> Result<Coverage, Error> {
        Coverage::new(
            [
                min,
                FramePoint([max.0[0], min.0[1]]),
                max,
                FramePoint([min.0[0], max.0[1]]),
            ]
            .map(|p| self.frame_to_source(p)),
        )
    }

    /// Snaps a frame-aligned crop proposal to source edges and constrains its coverage.
    pub fn crop_from_frame(self, min: FramePoint, max: FramePoint) -> Crop {
        let extent = std::array::from_fn::<_, 2, _>(|axis| {
            (max.0[axis] - min.0[axis]).abs().round().max(1.) as u32
        });
        let dimensions = self.size();
        let center = self
            .frame_to_source(FramePoint(std::array::from_fn(|i| {
                if min.0[i] == 0. {
                    f64::from(extent[i]) * 0.5
                } else if max.0[i] == f64::from(dimensions[i]) {
                    max.0[i] - f64::from(extent[i]) * 0.5
                } else {
                    (min.0[i] + max.0[i]) * 0.5
                }
            })))
            .0;
        let mut size = self.orientation.size_for(extent);
        if self.is_exact() {
            let scale = (f64::from(self.area().width) / f64::from(size[0]))
                .min(f64::from(self.area().height) / f64::from(size[1]))
                .min(1.);
            size = size.map(|v| (f64::from(v) * scale).round().max(1.) as u32);
        }
        Crop {
            center,
            size,
            angle: self.crop.angle,
        }
        .constrained(self.area())
        .expect("validated basis admits a positive frame")
    }

    /// Exact output-to-source pixel permutation, including the active-area origin.
    pub fn source_pixel(self, pixel: [u32; 2]) -> Result<[u32; 2], Error> {
        let size = self.size();
        if !self.is_exact() {
            return Err(Error::InvalidParameter(
                "rotated composition requires coverage reconstruction",
            ));
        }
        if pixel[0] >= size[0] || pixel[1] >= size[1] {
            return Err(Error::InvalidParameter("pixel outside composed frame"));
        }
        let point = self.frame_to_source(FramePoint([
            f64::from(pixel[0]) + 0.5,
            f64::from(pixel[1]) + 0.5,
        ]));
        Ok([point.0[0].floor() as u32, point.0[1].floor() as u32])
    }

    /// Converts an axis-aligned frame rectangle into source-texture UV coordinates.
    pub fn texture_mapping(
        self,
        min: FramePoint,
        max: FramePoint,
        sampled: SourceRegion,
    ) -> TextureMapping {
        let normalize = |point: FramePoint| {
            let source = self.frame_to_source(point);
            [
                (source.0[0] - f64::from(sampled.x)) / f64::from(sampled.width),
                (source.0[1] - f64::from(sampled.y)) / f64::from(sampled.height),
            ]
        };
        let origin = normalize(min);
        let right = normalize(FramePoint([max.0[0], min.0[1]]));
        let bottom = normalize(FramePoint([min.0[0], max.0[1]]));
        TextureMapping {
            origin: origin.map(|v| v as f32),
            x_axis: std::array::from_fn(|i| (right[i] - origin[i]) as f32),
            y_axis: std::array::from_fn(|i| (bottom[i] - origin[i]) as f32),
        }
    }
}

/// Affine normalized texture adapter: origin + u*x_axis + v*y_axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureMapping {
    pub origin: [f32; 2],
    pub x_axis: [f32; 2],
    pub y_axis: [f32; 2],
}

impl TextureMapping {
    pub const IDENTITY: Self = Self {
        origin: [0., 0.],
        x_axis: [1., 0.],
        y_axis: [0., 1.],
    };
}

impl From<[f32; 4]> for TextureMapping {
    fn from([x, y, width, height]: [f32; 4]) -> Self {
        Self {
            origin: [x, y],
            x_axis: [width, 0.],
            y_axis: [0., height],
        }
    }
}

#[cfg(test)]
mod tests;
