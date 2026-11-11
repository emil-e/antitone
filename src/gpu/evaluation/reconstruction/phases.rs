//! Reconstruction authors ordinary contribution/combine/output values; the compiler owns steps.
use super::*;
pub(super) const SOURCE_TILE: [u32; 2] = [512, 128];
pub(super) const PHASE_HEADROOM: u64 = 16 << 10;

impl Reconstruction {
    pub(super) fn express(
        &self,
        source: value::Calculation,
        domain: Domain,
        output: Rect,
        budget: Budget,
    ) -> Result<value::Calculation, Error> {
        use value::{Access as InputAccess, Calculation, Operation as Mathematics, Shape};
        let demand = self.reads(output)?[0].region;
        let cells = u64::from(output.width.checked_mul(output.height).ok_or(
            Error::InvalidParameter("evaluation raster exceeds GPU cell addressing"),
        )?);
        let available = budget
            .scratch_bytes
            .checked_sub(cells * 112 + PHASE_HEADROOM.min(budget.scratch_bytes / 4))
            .filter(|bytes| *bytes >= 64)
            .ok_or(Error::WorkingAllowance(
                "unit state exceeds the working allowance",
            ))?;
        let span = [
            self.packed.axes[0][0].abs() + self.packed.axes[1][0].abs(),
            self.packed.axes[0][1].abs() + self.packed.axes[1][1].abs(),
        ];
        let footprint = (0..2)
            .map(|axis| {
                (span[axis].ceil() + 2.).min(self.input.bounds.map_or(f64::INFINITY, |bounds| {
                    f64::from([bounds.width, bounds.height][axis])
                }))
            })
            .product::<f64>();

        let whole = if footprint <= 65_536.
            && u64::from(demand.width) * u64::from(demand.height) <= available / 64
            && demand.height as usize <= value::MAX_UNIT_VALUES
        {
            match self.source_parts(demand, output).as_slice() {
                [part] => Some(*part),
                _ => None,
            }
        } else {
            None
        };
        let source_cells =
            (available / 64).min(u64::from(SOURCE_TILE[0]) * u64::from(SOURCE_TILE[1]));
        let source_cells = 1u64 << source_cells.ilog2();
        let bounds = self.input.bounds.unwrap_or(demand);
        let mut width = u64::from(bounds.width)
            .min(u64::from(SOURCE_TILE[0]))
            .min(source_cells) as u32;
        let mut height = (source_cells / u64::from(width))
            .min(u64::from(SOURCE_TILE[1]))
            .min(u64::from(bounds.height)) as u32;
        let align = |start: i32, origin: i32, size: u32| {
            (i64::from(origin)
                + (i64::from(start) - i64::from(origin)).div_euclid(i64::from(size))
                    * i64::from(size)) as i32
        };
        let aligned = if let Some(part) = whole {
            width = part.width;
            height = part.height;
            part
        } else {
            let x = align(demand.x, bounds.x, width);
            let y = align(demand.y, bounds.y, height);
            Rect::new(
                [x, y],
                [(demand.right() - x) as u32, (demand.bottom() - y) as u32],
            )
        };
        let arguments = Domain {
            meaning: Meaning::Arguments,
            ..domain
        };
        let seed = "fn pixel(p:vec2i, position:vec2f)->array<vec4f,2>{return array<vec4f,2>(vec4f(0.0),vec4f(0.0));}";
        let mut sum = Calculation::new(
            arguments,
            output,
            vec![],
            Mathematics::new(
                "empty area contribution v1",
                (),
                seed,
                "pixel",
                InputAccess::Pointwise,
            )
            .with_shape(Shape::Pair),
        )?;
        let mut values = 2usize;
        let mut first = true;
        let mut partitions = 0usize;
        for chunk in tiled_units(aligned, [width, height])? {
            partitions += 1;
            if partitions > value::MAX_UNIT_VALUES {
                value::retire_gpu_graph(sum, &[source]);
                return Err(Error::WorkingAllowance(
                    "calculation exceeds the unit graph capacity",
                ));
            }
            let x = chunk.x.max(demand.x);
            let y = chunk.y.max(demand.y);
            let chunk = Rect::new(
                [x, y],
                [(chunk.right() - x) as u32, (chunk.bottom() - y) as u32],
            );
            for part in self.source_parts(chunk, output) {
                let Some(affected) = self.affected(part, output) else {
                    continue;
                };
                values += 2;
                if values > value::MAX_UNIT_VALUES {
                    value::retire_gpu_graph(sum, &[source]);
                    return Err(Error::WorkingAllowance(
                        "calculation exceeds the unit graph capacity",
                    ));
                }
                let mask = self.support_mask(affected, part);
                let admitted = if mask.is_some() {
                    "fn reconstruction_admitted(p:vec2i, geometry:Geometry)->bool {let local=vec2u(p)-geometry.sample_bounds.xy;let index=local.y*(geometry.sample_bounds.z-geometry.sample_bounds.x)+local.x;return (table_0(index/128u)[(index/32u)%4u]&(1u<<(index%32u)))!=0u;}"
                } else {
                    "fn reconstruction_admitted(p:vec2i, geometry:Geometry)->bool{return true;}"
                };
                let code=[
                    include_str!("../reconstruction_geometry.wgsl"),include_str!("../footprint.wgsl"),include_str!("../reconstruction_mean.wgsl"),
                    "fn reconstruction_sample(p:vec2i)->vec4f{return input_0(p);}",admitted,
                    include_str!("../reconstruction_contribution.wgsl"),
                    "fn pixel(p:vec2i, position:vec2f, geometry:Geometry)->array<vec4f,2>{let value=contribution(vec2u(p),geometry);return array<vec4f,2>(vec4f(value.mean,value.mass),vec4f(value.flags,0.0,0.0,0.0));}"
                ].join("\n");
                let work = footprint
                    .min(f64::from(part.width) * f64::from(part.height))
                    .ceil() as u64;
                let mut operation = Mathematics::new(
                    "area contribution v1",
                    (self.mapping, self.support, part, affected),
                    code,
                    "pixel",
                    InputAccess::Neighborhood,
                )
                .with_shape(Shape::Pair)
                .with_sampling_work(work)
                .with_uniform(
                    "Geometry",
                    &self.geometry_parameters(affected, output, part),
                );
                if let Some(mask) = mask {
                    let mut rows = vec![[0u32; 4]; mask.len().div_ceil(4)];
                    for (i, word) in mask.into_iter().enumerate() {
                        rows[i / 4][i % 4] = word;
                    }
                    operation = operation.with_table(rows);
                }
                let contribution =
                    Calculation::new(arguments, affected, vec![source.request(part)?], operation)?;
                sum = if first && affected == output {
                    contribution
                } else {
                    Calculation::new(
                        arguments,
                        output,
                        vec![sum, contribution],
                        Mathematics::new(
                            "ordered area combination v1",
                            affected,
                            [
                                include_str!("../reconstruction_mean.wgsl"),
                                include_str!("combine.wgsl"),
                            ]
                            .join("\n"),
                            "pixel",
                            InputAccess::Neighborhood,
                        )
                        .with_input_access(vec![InputAccess::Pointwise, InputAccess::Neighborhood])
                        .with_shape(Shape::Pair)
                        .with_uniform("Parameters", &affected.packed()),
                    )?
                };
                first = false;
            }
        }
        let code=[include_str!("../reconstruction_geometry.wgsl"),include_str!("../footprint.wgsl"),include_str!("../reconstruction_status.wgsl"),include_str!("../reconstruction_flags.wgsl"),
            "fn pixel(p:vec2i, position:vec2f, value:array<vec4f,2>, geometry:Geometry)->vec4f{return vec4f(value[0].rgb,covered_flags(value[1].x,value[0].w,geometry));}"].join("\n");
        let result = Calculation::new(
            domain,
            output,
            vec![sum],
            Mathematics::new(
                "covered area output v1",
                (self.mapping, self.support),
                code,
                "pixel",
                InputAccess::Pointwise,
            )
            .with_uniform(
                "Geometry",
                &self.geometry_parameters(output, output, demand),
            ),
        )?;
        Ok(result)
    }

    // A source unit can affect many output cells. Extend only units touching real source edges,
    // then conservatively project into output cell coordinates, including packed GPU arithmetic.
    pub(super) fn affected(&self, part: Rect, output: Rect) -> Option<Rect> {
        let b = self.input.bounds;
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        for map in [self.mapping, self.packed] {
            let inverse = DMat2::from_cols_array_2d(&map.axes).inverse();
            let corners = [
                [output.x, output.y],
                [output.right(), output.y],
                [output.right(), output.bottom()],
                [output.x, output.bottom()],
            ]
            .map(|p| map.point(p.map(f64::from)));
            let minimum: [f64; 2] =
                std::array::from_fn(|i| corners.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min));
            let maximum: [f64; 2] = std::array::from_fn(|i| {
                corners
                    .iter()
                    .map(|p| p[i])
                    .fold(f64::NEG_INFINITY, f64::max)
            });
            let first = [
                if b.is_some_and(|b| part.x == b.x) {
                    minimum[0].min(f64::from(part.x))
                } else {
                    f64::from(part.x)
                },
                if b.is_some_and(|b| part.y == b.y) {
                    minimum[1].min(f64::from(part.y))
                } else {
                    f64::from(part.y)
                },
            ];
            let end = [
                if b.is_some_and(|b| part.right() == b.right()) {
                    maximum[0].max(f64::from(part.right()))
                } else {
                    f64::from(part.right())
                },
                if b.is_some_and(|b| part.bottom() == b.bottom()) {
                    maximum[1].max(f64::from(part.bottom()))
                } else {
                    f64::from(part.bottom())
                },
            ];
            for p in [
                [first[0], first[1]],
                [end[0], first[1]],
                [end[0], end[1]],
                [first[0], end[1]],
            ] {
                let p = inverse * (DVec2::from(p) - DVec2::from(map.start));
                for i in 0..2 {
                    low[i] = low[i].min(p[i]);
                    high[i] = high[i].max(p[i]);
                }
            }
        }
        let x = (low[0].floor() - 1.)
            .max(f64::from(output.x))
            .min(f64::from(output.right())) as i32;
        let y = (low[1].floor() - 1.)
            .max(f64::from(output.y))
            .min(f64::from(output.bottom())) as i32;
        let right = (high[0].ceil() + 1.)
            .min(f64::from(output.right()))
            .max(f64::from(output.x)) as i32;
        let bottom = (high[1].ceil() + 1.)
            .min(f64::from(output.bottom()))
            .max(f64::from(output.y)) as i32;
        (right > x && bottom > y).then(|| [x, y, right - x, bottom - y].into())
    }
}
