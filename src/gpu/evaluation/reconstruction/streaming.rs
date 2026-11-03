//! Bounded original-cell contributions use the same integration kernel as a resident dispatch.
use super::*;

pub(super) mod batch;
#[cfg(all(test, feature = "editor"))]
mod tests;

impl Reconstruction {
    pub(super) fn produce_image(
        &self,
        worker: &Production<'_>,
        input: &Arc<ImageNode>,
    ) -> Result<(), Error> {
        let requested = worker.layout.region;
        let mut executor = worker.executor();
        for y in (requested.y..requested.bottom()).step_by(worker.budget.tile[1] as usize) {
            for x in (requested.x..requested.right()).step_by(worker.budget.tile[0] as usize) {
                if (worker.cancelled)() {
                    return Err(Error::Cancelled);
                }
                let region = Rect::new(
                    [x, y],
                    [
                        worker.budget.tile[0].min((requested.right() - x) as u32),
                        worker.budget.tile[1].min((requested.bottom() - y) as u32),
                    ],
                );
                // Resident input also needs bounded loops: unit weights stop advancing at 2^24.
                if self.requires_reduction() {
                    executor = worker.executor();
                    self.stream(worker, input, region, &mut executor)?;
                    continue;
                }
                let plan = self.plan(worker, input, region)?;
                if plan.peak_scratch_bytes <= worker.budget.scratch_bytes
                    && Executor::admit(&worker.context.device().limits(), &plan).is_ok()
                {
                    worker.deliver(&mut executor, plan)?;
                } else {
                    // No pixel acquisition/allocation has occurred while testing the whole demand.
                    drop(plan);
                    executor = worker.executor();
                    self.stream(worker, input, region, &mut executor)?;
                }
            }
        }
        Ok(())
    }

    fn stream(
        &self,
        worker: &Production<'_>,
        input: &Arc<ImageNode>,
        output: Rect,
        executor: &mut Executor,
    ) -> Result<(), Error> {
        batch::stream(&[(self, worker, output)], input, executor)
    }

    fn plan(
        &self,
        worker: &Production<'_>,
        input: &Arc<ImageNode>,
        region: Rect,
    ) -> Result<Plan, Error> {
        let mut fragment = Fragment::default();
        let demand = self.input(region)?.unwrap();
        let sample = super::super::plan::sample_region(
            input,
            demand,
            &mut fragment,
            super::super::plan::Planning {
                policy: Policy::Composed,
                scope: Some(worker.scope),
                supplied: &[],
                scratch_bytes: worker.budget.scratch_bytes,
                limits: Some(&worker.context.device().limits()),
            },
        )?;
        let output = fragment.image(worker.layout.domain, region);
        self.emit(&mut fragment, sample, output)?;
        Plan::from_work(fragment, output, u64::MAX)
    }

    fn idle_sample() -> Sampled {
        Sampled {
            code: "fn unused_sample(p: vec2i) -> vec4f { return vec4f(0.0); }".into(),
            symbol: "unused_sample".into(),
            bindings: vec![],
        }
    }

    fn clear_plan(
        &self,
        worker: &Production<'_>,
        output: Rect,
        scratch_bytes: u64,
    ) -> Result<Plan, Error> {
        let mut fragment = Fragment::default();
        let result = PlannedImage {
            domain: worker.layout.domain,
            region: output,
            resource: fragment.resource(Storage::Input {
                slot: 0,
                bytes: u64::from(output.width) * u64::from(output.height) * 16,
                writable: true,
            }),
        };
        let metadata = fragment.resource(Storage::Input {
            slot: 1,
            bytes: u64::from(output.width) * u64::from(output.height) * 4,
            writable: true,
        });
        fragment.compute(
            Self::code(&Self::idle_sample(), false),
            "clear",
            vec![
                (21, Binding::Write(result.resource)),
                (23, Binding::Write(metadata)),
            ],
            output.width * output.height,
        );
        Plan::from_work(fragment, result, scratch_bytes)
    }

    fn finish_plan(
        &self,
        worker: &Production<'_>,
        output: Rect,
        scratch_bytes: u64,
    ) -> Result<Plan, Error> {
        let mut fragment = Fragment::default();
        let source = fragment.resource(Storage::Input {
            slot: 0,
            bytes: u64::from(output.width) * u64::from(output.height) * 16,
            writable: false,
        });
        let metadata = fragment.resource(Storage::Input {
            slot: 1,
            bytes: u64::from(output.width) * u64::from(output.height) * 4,
            writable: true,
        });
        let result = PlannedImage {
            domain: worker.layout.domain,
            region: output,
            resource: fragment.resource(Storage::Input {
                slot: 2,
                bytes: worker.pixels.size(),
                writable: true,
            }),
        };
        let geometry = self.geometry(
            &mut fragment,
            output,
            worker.layout.region,
            self.input(output)?.unwrap(),
        );
        fragment.compute(
            Self::code(&Self::idle_sample(), false),
            "finish",
            vec![
                (21, Binding::Write(result.resource)),
                (22, geometry),
                (23, Binding::Write(metadata)),
                (24, Binding::Read(source)),
            ],
            output.width * output.height,
        );
        Plan::from_work(fragment, result, scratch_bytes)
    }

    // A source unit can affect many output cells. Extend only units touching real source edges,
    // then conservatively project into output cell coordinates, including packed GPU arithmetic.
    fn affected(&self, part: Rect, output: Rect) -> Option<Rect> {
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
