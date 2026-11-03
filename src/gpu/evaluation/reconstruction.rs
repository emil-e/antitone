//! Cell-area reconstruction between affine image grids, independent of photographic position.
use super::CellRegion as Rect;
use super::*;
use glam::{DMat2, DVec2};

#[cfg(test)]
mod fixture;
mod streaming;
#[cfg(test)]
pub(crate) use fixture::{encode_supplied, supplied_mask};
#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Mapping {
    pub start: [f64; 2],
    pub axes: [[f64; 2]; 2],
}

impl Mapping {
    fn point(self, point: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|i| {
            self.start[i] + point[0] * self.axes[0][i] + point[1] * self.axes[1][i]
        })
    }
    fn admit_region(self, region: Rect) -> Result<(), Error> {
        region.validate()?;
        let packed = self.packed();
        let extent = [
            region.x.unsigned_abs().max(region.right().unsigned_abs()),
            region.y.unsigned_abs().max(region.bottom().unsigned_abs()),
        ];
        let step = self.axes[0][0]
            .hypot(self.axes[0][1])
            .min(self.axes[1][0].hypot(self.axes[1][1]));
        for i in 0..2 {
            let magnitude = packed.start[i].abs()
                + (0..2)
                    .map(|axis| packed.axes[axis][i].abs() * f64::from(extent[axis]))
                    .sum::<f64>();
            if !magnitude.is_finite() || 8. * f64::from(f32::EPSILON) * magnitude >= step.min(1.) {
                return Err(Error::InvalidParameter(
                    "reconstruction exceeds GPU grid precision",
                ));
            }
        }
        Ok(())
    }

    fn packed(self) -> Self {
        Self {
            start: self.start.map(|v| f64::from(v as f32)),
            axes: self.axes.map(|v| v.map(|v| f64::from(v as f32))),
        }
    }
}

pub(crate) fn accumulate_supplied(
    image: &Arc<ImageNode>,
    fragment: &mut Fragment,
    input: Sampled,
    (samples, affected): (Rect, Rect),
    output: PlannedImage,
    metadata: Resource,
) -> Result<(), Error> {
    let operation = (image.operation.as_ref() as &dyn std::any::Any)
        .downcast_ref::<Reconstruction>()
        .ok_or(Error::InvalidParameter(
            "supplied accumulation requires a reconstruction",
        ))?;
    operation.accumulate(fragment, input, (samples, affected), output, metadata)
}

pub(crate) fn reconstruct(input: Arc<ImageNode>, output: Domain) -> Result<Arc<ImageNode>, Error> {
    reconstructed(input, output, false)
}

pub(crate) fn reconstruct_complete(
    input: Arc<ImageNode>,
    output: Domain,
) -> Result<Arc<ImageNode>, Error> {
    reconstructed(input, output, true)
}

fn reconstructed(
    input: Arc<ImageNode>,
    output: Domain,
    complete: bool,
) -> Result<Arc<ImageNode>, Error> {
    let source = DMat2::from_cols_array_2d(&input.domain.grid.axes);
    if !source.is_finite() || source.determinant() == 0. {
        return Err(Error::InvalidParameter("invalid reconstruction input grid"));
    }
    let inverse = source.inverse();
    let start = inverse * (DVec2::from(output.grid.origin) - DVec2::from(input.domain.grid.origin));
    let axes = output
        .grid
        .axes
        .map(|v| (inverse * DVec2::from(v)).to_array());
    mapped(
        input,
        output,
        Mapping {
            start: start.to_array(),
            axes,
        },
        complete,
    )
}

// Exact local-grid mappings avoid losing an integral padding offset through rounded world origins.
pub(crate) fn reconstruct_cells(
    input: Arc<ImageNode>,
    output: Domain,
    base: [f64; 2],
    scale: [f64; 2],
) -> Result<Arc<ImageNode>, Error> {
    reconstruct_mapped(
        input,
        output,
        Mapping {
            start: base,
            axes: [[scale[0], 0.], [0., scale[1]]],
        },
    )
}

pub(crate) fn reconstruct_mapped(
    input: Arc<ImageNode>,
    output: Domain,
    mapping: Mapping,
) -> Result<Arc<ImageNode>, Error> {
    mapped(input, output, mapping, false)
}

fn mapped(
    input: Arc<ImageNode>,
    output: Domain,
    mapping: Mapping,
    complete: bool,
) -> Result<Arc<ImageNode>, Error> {
    if let Some(bounds) = output.bounds {
        mapping.admit_region(bounds)?;
    }
    let packed = mapping.packed();
    let determinant =
        |map: Mapping| map.axes[0][0] * map.axes[1][1] - map.axes[0][1] * map.axes[1][0];
    if output.meaning != input.domain.meaning
        || (input.domain.grid.coverage != 0 && output.grid.coverage != input.domain.grid.coverage)
        || mapping
            .start
            .iter()
            .chain(mapping.axes.iter().flatten())
            .any(|v| !v.is_finite())
        || output.grid.pitch.iter().any(|v| !v.is_finite() || *v <= 0.)
        || !determinant(packed).is_finite()
        || determinant(packed) == 0.
    {
        return Err(Error::InvalidParameter("invalid reconstruction mapping"));
    }
    let origin = input.domain.grid.point(mapping.start);
    let axes: [[f64; 2]; 2] = mapping.axes.map(|v| {
        std::array::from_fn(|i| {
            v[0] * input.domain.grid.axes[0][i] + v[1] * input.domain.grid.axes[1][i]
        })
    });
    let close = |a: f64, b: f64, magnitude: f64| {
        a.is_finite()
            && b.is_finite()
            && (a - b).abs()
                <= 8. * f64::from(f32::EPSILON) * magnitude.max(a.abs()).max(b.abs()).max(1.)
    };
    for i in 0..2 {
        let magnitude = input.domain.grid.origin[i].abs()
            + (0..2)
                .map(|axis| (mapping.start[axis] * input.domain.grid.axes[axis][i]).abs())
                .sum::<f64>();
        if !close(origin[i], output.grid.origin[i], magnitude)
            || (0..2).any(|a| !close(axes[a][i], output.grid.axes[a][i], 0.))
        {
            return Err(Error::InvalidParameter(
                "cell mapping differs from declared grids",
            ));
        }
        let length = axes[i][0].hypot(axes[i][1]);
        if !close(length, output.grid.pitch[i], 0.) {
            return Err(Error::InvalidParameter(
                "cell pitch differs from declared grid",
            ));
        }
    }
    Ok(ImageNode::new(
        output,
        Some(input.clone()),
        Reconstruction {
            input: input.domain,
            mapping,
            packed,
            support: if complete {
                let bounds = output.finite()?;
                Some(Support {
                    output: bounds,
                    coverage: crate::coverage::Coverage::new(
                        [
                            [bounds.x, bounds.y],
                            [bounds.right(), bounds.y],
                            [bounds.right(), bounds.bottom()],
                            [bounds.x, bounds.bottom()],
                        ]
                        .map(|p| crate::geometry::SourcePoint(mapping.point(p.map(f64::from)))),
                    )?,
                })
            } else {
                None
            },
            coverage: if input.domain.grid.coverage == 0 {
                output.grid.coverage
            } else {
                0
            },
        },
    ))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Support {
    output: Rect,
    coverage: crate::coverage::Coverage,
}

struct Reconstruction {
    input: Domain,
    mapping: Mapping,
    packed: Mapping,
    coverage: u32,
    support: Option<Support>,
}
impl Reconstruction {
    fn accumulate(
        &self,
        fragment: &mut Fragment,
        input: Sampled,
        (samples, affected): (Rect, Rect),
        output: PlannedImage,
        metadata: Resource,
    ) -> Result<(), Error> {
        let geometry = self.geometry(fragment, affected, output.region, samples);
        let support = self.support_binding(fragment, affected, samples);
        let code = Reconstruction::code(&input, support.is_some());
        let mut bindings = input.bindings;
        if let Some(support) = support {
            bindings.push((25, support));
        }
        bindings.extend([
            (21, Binding::Write(output.resource)),
            (22, geometry),
            (23, Binding::Write(metadata)),
        ]);
        fragment.compute(
            code,
            "accumulate",
            bindings,
            affected.width * affected.height,
        );
        Ok(())
    }

    fn requires_reduction(&self) -> bool {
        let width = self.packed.axes[0][0].abs() + self.packed.axes[1][0].abs();
        let height = self.packed.axes[0][1].abs() + self.packed.axes[1][1].abs();
        (width.ceil() + 2.).min(
            self.input
                .bounds
                .map_or(f64::INFINITY, |bounds| f64::from(bounds.width)),
        ) * (height.ceil() + 2.).min(
            self.input
                .bounds
                .map_or(f64::INFINITY, |bounds| f64::from(bounds.height)),
        ) > 65536.
    }

    fn code(input: &Sampled, masked: bool) -> Arc<str> {
        let support = if masked {
            r#"@group(0) @binding(25) var<storage,read> reconstruction_support:array<u32>;
fn reconstruction_admitted(p:vec2i)->bool {
    let local=vec2u(p)-geometry.sample_bounds.xy;
    let index=local.y*(geometry.sample_bounds.z-geometry.sample_bounds.x)+local.x;
    return (reconstruction_support[index/32u]&(1u<<(index%32u)))!=0u;
}"#
        } else {
            "fn reconstruction_admitted(p:vec2i)->bool {return true;}"
        };
        format!(
            "{}\n{}\nfn reconstruction_sample(p: vec2i) -> vec4f {{ return {}(p); }}\n{}\n{}",
            include_str!("../../shaders/footprint.wgsl"),
            input.code,
            input.symbol,
            support,
            [
                include_str!("reconstruction_status.wgsl"),
                include_str!("reconstruct.wgsl")
            ]
            .join("\n"),
        )
        .into()
    }

    fn support_binding(
        &self,
        fragment: &mut Fragment,
        output: Rect,
        samples: Rect,
    ) -> Option<Binding> {
        let support = self.support?;
        let b = support.output;
        if self
            .input
            .bounds
            .and_then(|bounds| bounds.source().ok())
            .is_some_and(|bounds| support.coverage.exact_rectangle(bounds) == Some(bounds))
        {
            return None;
        }
        if output.x > b.x
            && output.y > b.y
            && output.right() < b.right()
            && output.bottom() < b.bottom()
        {
            return None;
        }
        let mut mask = vec![
            0u32;
            (u64::from(samples.width) * u64::from(samples.height)).div_ceil(32)
                as usize
        ];
        for y in samples.y..samples.bottom() {
            for x in samples.x..samples.right() {
                if support
                    .coverage
                    .rectangle_weight([f64::from(x), f64::from(y)], [1.; 2])
                    > 0.
                {
                    let index = (y - samples.y) as u32 * samples.width + (x - samples.x) as u32;
                    mask[index as usize / 32] |= 1 << (index % 32);
                }
            }
        }
        Some(Binding::Read(
            fragment.resource(Storage::Data(bytemuck::cast_slice(&mask).into())),
        ))
    }

    fn geometry(
        &self,
        fragment: &mut Fragment,
        output: Rect,
        whole: Rect,
        samples: Rect,
    ) -> Binding {
        let source_bounds = self.input.bounds.map_or([0; 4], |b| {
            [b.x as u32, b.y as u32, b.right() as u32, b.bottom() as u32]
        });
        let integers = [
            [self.coverage, whole.width, output.width, output.height],
            [
                whole.x as u32,
                whole.y as u32,
                output.x as u32,
                output.y as u32,
            ],
            source_bounds,
            [
                samples.x as u32,
                samples.y as u32,
                samples.right() as u32,
                samples.bottom() as u32,
            ],
        ];
        let start = self
            .mapping
            .point([f64::from(output.x), f64::from(output.y)]);
        let mut bytes = bytemuck::cast_slice(&integers).to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(&[
            (start[0] - f64::from(samples.x)) as f32,
            (start[1] - f64::from(samples.y)) as f32,
            self.mapping.axes[0][0] as f32,
            self.mapping.axes[0][1] as f32,
            self.mapping.axes[1][0] as f32,
            self.mapping.axes[1][1] as f32,
            if self.support.is_some() { 1. } else { 0. },
            if self.input.bounds.is_some() { 1. } else { 0. },
        ]));
        Binding::Uniform(fragment.resource(Storage::Uniform(bytes.into())))
    }
}

impl Operation for Reconstruction {
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new((self.mapping, self.support)))
    }

    fn sample_consumer(&self) -> Option<&dyn SampleConsumer> {
        Some(self)
    }
    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        self.mapping.admit_region(region)?;
        let x = [f64::from(region.x), f64::from(region.right())];
        let y = [f64::from(region.y), f64::from(region.bottom())];
        let corners = [[x[0], y[0]], [x[1], y[0]], [x[1], y[1]], [x[0], y[1]]];
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        for map in [self.mapping, self.packed] {
            for corner in corners {
                let p = map.point(corner);
                for i in 0..2 {
                    low[i] = low[i].min(p[i]);
                    high[i] = high[i].max(p[i]);
                }
            }
        }
        // Integral mappings are exact within the admitted f32 coordinate range.
        // Their half-open support needs no extra roundoff halo.
        let guard = if self
            .mapping
            .start
            .iter()
            .chain(self.mapping.axes.iter().flatten())
            .all(|value| value.fract() == 0.)
        {
            0.
        } else {
            1.
        };
        for i in 0..2 {
            low[i] = low[i].floor() - guard;
            high[i] = high[i].ceil() + guard;
            if let Some(bounds) = self.input.bounds {
                let minimum = [bounds.x, bounds.y];
                let maximum = [bounds.right(), bounds.bottom()];
                low[i] = low[i].clamp(f64::from(minimum[i]), f64::from(maximum[i] - 1));
                high[i] = high[i].clamp(f64::from(minimum[i] + 1), f64::from(maximum[i]));
            }
            if !low[i].is_finite()
                || !high[i].is_finite()
                || low[i] < f64::from(i32::MIN)
                || high[i] > f64::from(i32::MAX)
                || high[i] <= low[i]
            {
                return Err(Error::InvalidParameter(
                    "reconstruction neighbourhood exceeds coordinate range",
                ));
            }
        }
        let low = low.map(|v| v as i32);
        let high = high.map(|v| v as i32);
        Ok(Some(
            Rect::new(
                low,
                [
                    (i64::from(high[0]) - i64::from(low[0])) as u32,
                    (i64::from(high[1]) - i64::from(low[1])) as u32,
                ],
            )
            .validate()?,
        ))
    }
    fn lower(
        &self,
        fragment: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let sample = Sampled::raster(fragment, input.unwrap());
        self.emit(fragment, sample, output)
    }
}
impl SampleProduction for Reconstruction {
    fn requires_worker(&self) -> bool {
        self.requires_reduction()
    }

    fn produce_many(
        &self,
        outputs: &[(&Arc<ImageNode>, Production<'_>)],
    ) -> Result<Option<Stats>, Error> {
        streaming::batch::produce(outputs)
    }

    fn produce(&self, worker: &Production<'_>, input: &Arc<ImageNode>) -> Result<Stats, Error> {
        self.produce_image(worker, input)
    }
}

impl SampleConsumer for Reconstruction {
    fn production(&self) -> Option<&dyn SampleProduction> {
        Some(self)
    }

    fn emit(
        &self,
        fragment: &mut Fragment,
        input: Sampled,
        output: PlannedImage,
    ) -> Result<(), Error> {
        if self.requires_reduction() {
            return Err(Error::InvalidParameter(
                "reconstruction requires bounded worker production",
            ));
        }
        let samples = self.input(output.region)?.unwrap();
        let support = self.support_binding(fragment, output.region, samples);
        let uniform = self.geometry(fragment, output.region, output.region, samples);
        let code = Self::code(&input, support.is_some());
        let mut bindings = input.bindings;
        if let Some(support) = support {
            if bindings.iter().any(|(slot, _)| *slot == 25) {
                return Err(Error::InvalidParameter(
                    "sample code collides with reconstruction support ABI",
                ));
            }
            bindings.push((25, support));
        }
        bindings.extend([(21, Binding::Write(output.resource)), (22, uniform)]);
        fragment.compute(code, "main", bindings, output.cells());
        Ok(())
    }
}
