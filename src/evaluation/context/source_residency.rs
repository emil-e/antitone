//! Select optional source storage for a complete evaluation demand, before tiled execution.
use super::*;
use std::sync::OnceLock;

#[cfg(all(test, feature = "editor"))]
mod tests;

/// An execution's input leases are independent of optional retention. Related
/// deliveries may run in stages without reacquiring their shared source.
#[derive(Clone, Default)]
pub(crate) struct EvaluationRun(Arc<OnceLock<Option<Image<ImageLayout>>>>);

impl EvaluationRun {
    pub(super) fn prepare_sources(
        &self,
        context: &Context,
        queue: &crate::gpu::submission::Queue,
        evaluation: &Evaluation,
        cancelled: &dyn Fn() -> bool,
    ) -> bool {
        self.0
            .get_or_init(|| {
                let root = evaluation.image();
                let mut sources = root.ancestors().filter(|value| value.source().is_some());
                let source = sources.next()?;
                // At most one full texture per run. Multiple independent sources
                // still use the ordinary bounded rectangle acquisition.
                if sources.any(|other| !other.same_definition(source)) {
                    return None;
                }
                let scope = context.materializations();
                let upload = context.resident_source_texture(
                    queue,
                    source,
                    |layout| {
                        let demand = evaluation.region();
                        admitted(layout, demand) && needs_source(&scope, root, demand.bounds)
                    },
                    cancelled,
                );
                match upload {
                    Ok(texture) => texture,
                    Err(Error::Cancelled) => None,
                    Err(error) => {
                        log::debug!(error = error.to_string().as_str(); "resident_source_unavailable");
                        None
                    }
                }
            })
            .is_some()
    }
}

fn admitted(source: ImageLayout, demand: Region) -> bool {
    // Compare the requested footprint on the shared source-coordinate grid.
    // This is an upload heuristic, not a bound on any calculation's actual reads.
    if source.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        || !(32 << 20..=768 << 20).contains(&source.bytes())
    {
        return false;
    }
    let bounds = demand.bounds;
    let points = [
        [bounds.x, bounds.y],
        [bounds.right(), bounds.y],
        [bounds.right(), bounds.bottom()],
        [bounds.x, bounds.bottom()],
    ]
    .map(|p| crate::geometry::SourcePoint(demand.grid.point(p.map(f64::from))));
    let coverage = source
        .region
        .source()
        .and_then(|active| crate::coverage::Coverage::new(points)?.clipped_bounds(active));
    let cells = u64::from(source.region.width) * u64::from(source.region.height);
    coverage.is_ok_and(|area| u64::from(area.width) * u64::from(area.height) >= cells.div_ceil(2))
}

fn needs_source(scope: &Materializations, root: &Value, region: CellRegion) -> bool {
    // Admission is advisory. Follow declared reads, stopping at stored inputs.
    // Unavailable descriptions or a large probe can keep the ordinary plan.
    let mut pending = vec![(root, region)];
    for _ in 0..value::MAX_UNIT_VALUES {
        let Some((node, region)) = pending.pop() else {
            break;
        };
        match super::super::materialized::Stored::find(Some(scope), &[], node, region) {
            Ok(Some(_)) | Err(_) => continue,
            Ok(None) if node.source().is_some() => return true,
            Ok(None) => {}
        }
        if let Ok(reads) = node.operation().reads(region) {
            pending.extend(reads.into_iter().rev().filter_map(|read| {
                let input = node.inputs().get(read.input)?;
                Some((input, read.region))
            }));
        }
    }
    false
}

/// Lazy bounded planning of the subregions of one captured demand, in sink order.
pub(crate) struct Traversal {
    evaluation: Evaluation,
    _run: EvaluationRun,
}

impl Traversal {
    pub(crate) fn plan(&self, region: CellRegion) -> Result<Plan, Error> {
        let job = &self.evaluation.job;
        if !job.region().contains_region(region)
            || region.width > job.tiling().tile[0]
            || region.height > job.tiling().tile[1]
        {
            return Err(Error::InvalidParameter("invalid traversal unit"));
        }
        job.probe(region)
    }
}

impl Context {
    /// Prepare a bounded traversal on the existing worker, without queueing child work.
    pub(crate) fn traversal(
        &self,
        queue: &crate::gpu::submission::Queue,
        evaluation: Evaluation,
        check: impl Fn() -> Result<(), Error>,
    ) -> Result<Traversal, Error> {
        check()?;
        let stopped = std::cell::RefCell::new(None);
        let run = EvaluationRun::default();
        run.prepare_sources(self, queue, &evaluation, &|| {
            let mut stopped = stopped.borrow_mut();
            if stopped.is_none() {
                *stopped = check().err();
            }
            stopped.is_some()
        });
        if let Some(error) = stopped.into_inner() {
            return Err(error);
        }
        Ok(Traversal {
            evaluation,
            _run: run,
        })
    }
}
