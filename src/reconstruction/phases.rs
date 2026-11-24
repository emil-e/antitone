//! Reconstruction authors ordinary contribution/combine/output values; the compiler owns steps.
use super::*;

fn affine_axes(mapping: Mapping, output: Rect) -> Vec<[u32; 4]> {
    // Separate integer position from fractions before narrowing to f32. Each
    // coordinate is evaluated at its absolute output index, not a block offset.
    let pack = |position: [f64; 2]| {
        let cell = position.map(f64::floor);
        [
            cell[0] as i32 as u32,
            cell[1] as i32 as u32,
            ((position[0] - cell[0]) as f32).to_bits(),
            ((position[1] - cell[1]) as f32).to_bits(),
        ]
    };
    let x = (output.x..output.right()).map(|x| {
        pack(std::array::from_fn(|i| {
            mapping.start[i] + f64::from(x) * mapping.axes[0][i]
        }))
    });
    let y = (output.y..output.bottom()).map(|y| pack(mapping.axes[1].map(|v| f64::from(y) * v)));
    x.chain(y).collect()
}

impl Reconstruction {
    pub(super) fn express(
        &self,
        source: value::Calculation,
        domain: Domain,
        output: Rect,
        limits: Option<&wgpu::Limits>,
    ) -> Result<value::Calculation, Error> {
        use value::{Access as InputAccess, Calculation, Operation as Mathematics, Shape};
        let demand = self.input_demand(output)?;
        let aligned_inputs = self.tiled;
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

        let preferred = if aligned_inputs {
            [128, 32]
        } else {
            SOURCE_TILE
        };
        let source_cells = u64::from(preferred[0]) * u64::from(preferred[1]);
        let source_cells = 1u64 << source_cells.ilog2();
        let extent = match self.input.bounds {
            Some(bounds) => [bounds.width, bounds.height],
            None if aligned_inputs => preferred,
            None => [demand.width, demand.height],
        };
        let width = u64::from(extent[0])
            .min(u64::from(preferred[0]))
            .min(source_cells) as u32;
        let height = (source_cells / u64::from(width))
            .min(u64::from(preferred[1]))
            .min(u64::from(extent[1])) as u32;
        let whole = if footprint <= 65_536. && demand.height as usize <= value::MAX_UNIT_VALUES {
            match self.source_window(demand, output) {
                Some(part) => {
                    let fetched = if aligned_inputs {
                        self.covering_slices(part, [width, height])?
                    } else {
                        part
                    };
                    // Coalescing must not make otherwise representable contributions fail.
                    let bytes = u64::from(fetched.width) * u64::from(fetched.height) * 16;
                    limits
                        .is_none_or(|limits| {
                            bytes <= limits.max_buffer_size
                                && bytes <= limits.max_storage_buffer_binding_size
                        })
                        .then_some((fetched, part))
                }
                None => None,
            }
        } else {
            None
        };
        let bounds = self.input.bounds.unwrap_or(demand);
        let align = |start: i32, origin: i32, size: u32| {
            (i64::from(origin)
                + (i64::from(start) - i64::from(origin)).div_euclid(i64::from(size))
                    * i64::from(size)) as i32
        };
        let x = align(demand.x, bounds.x, width);
        let y = align(demand.y, bounds.y, height);
        let aligned = Rect::new(
            [x, y],
            [(demand.right() - x) as u32, (demand.bottom() - y) as u32],
        );
        let seed = "fn pixel(p:vec2i, position:vec2f)->array<vec4f,2>{return array<vec4f,2>(vec4f(0.0),vec4f(0.0));}";
        let mut sum = Calculation::new(
            domain,
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
        let chunks: WorkUnits = if let Some((fetched, _)) = whole {
            Box::new(std::iter::once(fetched))
        } else if aligned_inputs {
            self.input_slices(demand, [width, height])?
        } else {
            tiled_units(aligned, [width, height])?
        };
        for chunk in chunks {
            partitions += 1;
            if partitions > value::MAX_UNIT_VALUES {
                value::retire_gpu_graph(sum, &[source]);
                return Err(Error::EvaluationLimit(
                    "calculation exceeds the unit graph capacity",
                ));
            }
            let part = if let Some((_, part)) = whole {
                Some(part)
            } else if aligned_inputs {
                // Keep contribution geometry tight, independently of the fetched slice.
                self.source_window(chunk, output)
            } else {
                let x = chunk.x.max(demand.x);
                let y = chunk.y.max(demand.y);
                let clipped = Rect::new(
                    [x, y],
                    [(chunk.right() - x) as u32, (chunk.bottom() - y) as u32],
                );
                self.source_window(clipped, output)
            };
            if let Some(part) = part {
                let Some(affected) = self.affected(part, output) else {
                    continue;
                };
                values += 2;
                if values > value::MAX_UNIT_VALUES {
                    value::retire_gpu_graph(sum, &[source]);
                    return Err(Error::EvaluationLimit(
                        "calculation exceeds the unit graph capacity",
                    ));
                }
                let mask = self.support_mask(affected, part);
                let admitted = if mask.is_some() {
                    "fn reconstruction_admitted(p:vec2i, geometry:Geometry)->bool {let local=vec2u(p)-geometry.sample_bounds.xy;let index=local.y*(geometry.sample_bounds.z-geometry.sample_bounds.x)+local.x;return (table_0(index/128u)[(index/32u)%4u]&(1u<<(index%32u)))!=0u;}"
                } else {
                    "fn reconstruction_admitted(p:vec2i, geometry:Geometry)->bool{return true;}"
                };
                let affine = self.support.is_some() && !self.mapping.integral();
                let (axis, affine_body, integrate) = if affine {
                    let axis = if mask.is_some() {
                        "fn affine_axis(i: u32) -> vec4u { return table_1(i); }"
                    } else {
                        "fn affine_axis(i: u32) -> vec4u { return table_0(i); }"
                    };
                    (
                        axis,
                        include_str!("affine_contribution.wgsl"),
                        "affine_contribution(p, geometry)",
                    )
                } else {
                    ("", "", "block_contribution(vec2u(p), geometry)")
                };
                let endpoint = format!(
                    "fn pixel(p: vec2i, position: vec2f, geometry: Geometry) -> array<vec4f, 2> {{
                        let value = {integrate};
                        return array<vec4f, 2>(vec4f(value.mean, value.mass), vec4f(value.flags, 0.0, 0.0, 0.0));
                    }}"
                );
                let code = [
                    include_str!("reconstruction_geometry.wgsl"),
                    include_str!("footprint.wgsl"),
                    include_str!("reconstruction_mean.wgsl"),
                    "fn reconstruction_sample(p: vec2i) -> vec4f { return input_0(p); }",
                    admitted,
                    include_str!("reconstruction_contribution.wgsl"),
                    axis,
                    affine_body,
                    &endpoint,
                ]
                .join("\n");
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
                if affine {
                    operation = operation.with_table(affine_axes(self.mapping, affected));
                }
                let input = source.request(if aligned_inputs { chunk } else { part })?;
                let contribution = Calculation::new(domain, affected, vec![input], operation)?;
                sum = if first && affected == output {
                    contribution
                } else {
                    Calculation::new(
                        domain,
                        output,
                        vec![sum, contribution],
                        Mathematics::new(
                            "ordered area combination v1",
                            affected,
                            [
                                include_str!("reconstruction_mean.wgsl"),
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
        let code=[include_str!("reconstruction_geometry.wgsl"),include_str!("footprint.wgsl"),include_str!("reconstruction_status.wgsl"),include_str!("reconstruction_flags.wgsl"),
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
