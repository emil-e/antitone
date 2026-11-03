use super::*;

pub(in crate::gpu::evaluation::reconstruction) fn produce(
    outputs: &[(&Arc<ImageNode>, Production<'_>)],
) -> Result<Option<Stats>, Error> {
    let Some((first, _)) = outputs.first() else {
        return Ok(None);
    };
    let Some(input) = first.input.as_ref() else {
        return Ok(None);
    };
    let mut operations = Vec::new();
    for (image, worker) in outputs {
        let Some(operation) =
            (image.operation.as_ref() as &dyn std::any::Any).downcast_ref::<Reconstruction>()
        else {
            return Ok(None);
        };
        let Some(upstream) = image.input.as_ref() else {
            return Ok(None);
        };
        if !(Arc::ptr_eq(input, upstream)
            || input.identity().is_some() && input.identity() == upstream.identity())
            || worker.budget.tile[0] < worker.layout.region.width
            || worker.budget.tile[1] < worker.layout.region.height
        {
            return Ok(None);
        }
        operations.push(operation);
    }
    if outputs.len() < 2 {
        return Ok(None);
    }
    let mut plans = Vec::new();
    let mut resident = true;
    for (operation, (_, worker)) in operations.iter().zip(outputs) {
        if operation.requires_reduction() {
            resident = false;
            break;
        }
        let plan = operation.plan(worker, input, worker.layout.region)?;
        resident &= plan.peak_scratch_bytes <= worker.budget.scratch_bytes
            && Executor::admit(&worker.context.device().limits(), &plan).is_ok()
            && !plan
                .tiles
                .iter()
                .flat_map(|t| &t.fragment.work)
                .any(|w| matches!(w, Work::Upload { .. } | Work::Produce { .. }));
        plans.push(plan);
        if !resident {
            break;
        }
    }
    let mut executor = outputs[0].1.executor();
    let mut stats = Stats::default();
    if resident {
        for ((_, worker), plan) in outputs.iter().zip(plans) {
            stats.include(worker.deliver(&mut executor, plan)?, 0);
        }
    } else {
        drop(plans);
        let targets: Vec<_> = operations
            .iter()
            .zip(outputs)
            .map(|(op, (_, worker))| (*op, worker, worker.layout.region))
            .collect();
        stats.include(stream(&targets, input, &mut executor)?, 0);
    }
    Ok(Some(stats))
}

struct Reduction<'a, 'b> {
    operation: &'a Reconstruction,
    worker: &'a Production<'b>,
    region: Rect,
    means: wgpu::Buffer,
    flags: wgpu::Buffer,
    finish: Plan,
}
impl Reduction<'_, '_> {
    fn accumulation(&self, domain: Domain, part: Rect, affected: Rect) -> Result<Plan, Error> {
        let mut fragment = Fragment::default();
        let source = PlannedImage {
            domain,
            region: part,
            resource: fragment.resource(Storage::Input {
                slot: 0,
                bytes: u64::from(part.width) * u64::from(part.height) * 16,
                writable: false,
            }),
        };
        let sample = Sampled::raster(&mut fragment, source);
        let result = PlannedImage {
            domain: self.worker.layout.domain,
            region: self.region,
            resource: fragment.resource(Storage::Input {
                slot: 1,
                bytes: self.means.size(),
                writable: true,
            }),
        };
        let metadata = fragment.resource(Storage::Input {
            slot: 2,
            bytes: self.flags.size(),
            writable: true,
        });
        self.operation
            .accumulate(&mut fragment, sample, (part, affected), result, metadata)?;
        Plan::from_work(fragment, result, u64::MAX)
    }
}

pub(super) fn stream(
    outputs: &[(&Reconstruction, &Production<'_>, Rect)],
    input: &Arc<ImageNode>,
    executor: &mut Executor,
) -> Result<Stats, Error> {
    let worker = outputs[0].1;
    let device = worker.context.device();
    let limits = device.limits();
    let limit = limits
        .max_storage_buffer_binding_size
        .min(limits.max_buffer_size);
    let scratch: u64 = outputs
        .iter()
        .map(|(_, _, r)| u64::from(r.width) * u64::from(r.height) * 20)
        .sum();
    let budget = outputs
        .iter()
        .map(|(_, w, _)| w.budget.scratch_bytes)
        .min()
        .unwrap();
    let available =
        budget
            .checked_sub(scratch)
            .filter(|v| *v >= 64)
            .ok_or(Error::InvalidParameter(
                "reconstruction exceeds working storage",
            ))?;
    for (_, _, r) in outputs {
        if u64::from(r.width) * u64::from(r.height) * 16 > limit {
            return Err(Error::InvalidParameter(
                "reconstruction exceeds GPU storage limits",
            ));
        }
    }
    let mut demand: Option<Rect> = None;
    for (operation, _, region) in outputs {
        let next = operation.input(*region)?.unwrap();
        demand = Some(if let Some(previous) = demand {
            let x = previous.x.min(next.x);
            let y = previous.y.min(next.y);
            [
                x,
                y,
                (previous.right()).max(next.right()) - x,
                (previous.bottom()).max(next.bottom()) - y,
            ]
            .into()
        } else {
            next
        });
    }
    let demand = demand.unwrap();
    let source_cells = (available.min(limit) / 64).min(512 * 128);
    let width = u64::from(demand.width).min(512).min(source_cells) as u32;
    let height = (source_cells / u64::from(width))
        .min(128)
        .min(u64::from(demand.height)) as u32;
    let mut stats = Stats {
        buffer_allocations: 2 * outputs.len(),
        scratch_bytes: scratch,
        ..Default::default()
    };
    let buffer = |bytes| {
        #[cfg(all(test, feature = "editor"))]
        crate::gpu::test_support::evaluation_buffer_created(bytes);
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("required reconstruction reduction"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    };
    let targets = outputs
        .iter()
        .map(|(operation, worker, region)| {
            let clear = operation.clear_plan(worker, *region, available)?;
            let finish = operation.finish_plan(worker, *region, available)?;
            Executor::admit(&limits, &clear)?;
            Executor::admit(&limits, &finish)?;
            let means = buffer(u64::from(region.width) * u64::from(region.height) * 16);
            let flags = buffer(u64::from(region.width) * u64::from(region.height) * 4);
            stats.include(worker.submit_work(clear, &[&means, &flags])?, scratch);
            (worker.progress)();
            Ok(Reduction {
                operation,
                worker,
                region: *region,
                means,
                flags,
                finish,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let pending_uploads: u64 = targets
        .iter()
        .map(|target| {
            target
                .finish
                .tiles
                .iter()
                .map(|tile| tile.fragment.upload_bytes())
                .sum::<u64>()
        })
        .sum();
    stats.host_upload_bytes = stats.host_upload_bytes.max(pending_uploads);
    for y in (demand.y..demand.bottom()).step_by(height as usize) {
        for x in (demand.x..demand.right()).step_by(width as usize) {
            let part = Rect::new(
                [x, y],
                [
                    width.min((demand.right() - x) as u32),
                    height.min((demand.bottom() - y) as u32),
                ],
            );
            let mut pending = vec![part];
            while let Some(part) = pending.pop() {
                if (worker.cancelled)() {
                    return Err(Error::Cancelled);
                }
                let affected: Vec<_> = targets
                    .iter()
                    .filter_map(|target| {
                        target
                            .operation
                            .affected(part, target.region)
                            .map(|r| (target, r))
                    })
                    .collect();
                if affected.is_empty() {
                    continue;
                }
                let mut accumulations = affected
                    .iter()
                    .map(|(target, region)| {
                        let plan = target.accumulation(input.domain, part, *region)?;
                        Executor::admit(&limits, &plan)?;
                        Ok((*target, plan))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                let sink_bytes = accumulations
                    .iter()
                    .map(|(_, plan)| plan.peak_scratch_bytes)
                    .sum();
                let sink_uploads: u64 = accumulations
                    .iter()
                    .flat_map(|(_, plan)| &plan.tiles)
                    .map(|tile| tile.fragment.upload_bytes())
                    .sum();
                let source_budget = available.checked_sub(sink_bytes);
                let allowance = source_budget.unwrap_or(0);
                let mut plan = super::super::super::plan::probe(
                    input,
                    part,
                    Budget {
                        tile: [part.width, part.height],
                        scratch_bytes: allowance,
                    },
                    worker.scope,
                    &limits,
                )?;
                let admission = Executor::admit(&limits, &plan);
                if source_budget.is_none()
                    || plan.peak_scratch_bytes > allowance
                    || admission.is_err()
                {
                    if part.width == 1 && part.height == 1 {
                        admission?;
                        return Err(Error::InvalidParameter(
                            "reconstruction neighbourhood exceeds working storage",
                        ));
                    }
                    if part.width >= part.height && part.width > 1 {
                        let width = part.width / 2;
                        pending.extend([
                            Rect {
                                x: part.x + width as i32,
                                width: part.width - width,
                                ..part
                            },
                            Rect { width, ..part },
                        ]);
                    } else {
                        let height = part.height / 2;
                        pending.extend([
                            Rect {
                                y: part.y + height as i32,
                                height: part.height - height,
                                ..part
                            },
                            Rect { height, ..part },
                        ]);
                    }
                    continue;
                }
                plan.scratch_budget = allowance;
                let mut sinks = Stats::default();
                let mut live_sinks = 0;
                let mut source_stats = worker
                    .run_sink(executor, &plan, |encoder, source, pixels| {
                        debug_assert_eq!(source.region, part);
                        for (target, mut plan) in accumulations.drain(..) {
                            plan.scratch_budget = plan.peak_scratch_bytes;
                            let recorded = target.worker.executor().prepare(device, plan)?;
                            let mut encoded = recorded.encode_work(
                                device,
                                encoder,
                                &[pixels, &target.means, &target.flags],
                            )?;
                            encoded.include(recorded.preparation.clone(), 0);
                            let bytes = encoded.scratch_bytes;
                            sinks.include(encoded, live_sinks);
                            live_sinks += bytes;
                        }
                        Ok(())
                    })?
                    .ok_or(Error::Cancelled)?;
                source_stats.host_upload_bytes += pending_uploads + sink_uploads;
                source_stats.include(sinks, executor.pool_bytes());
                stats.include(source_stats, scratch);
                (worker.progress)();
            }
        }
    }
    // Source scratch is no longer live during the separately admitted final readouts.
    *executor = worker.executor();
    for target in targets {
        stats.include(
            target.worker.submit_work(
                target.finish,
                &[&target.means, &target.flags, target.worker.pixels],
            )?,
            scratch,
        );
    }
    Ok(stats)
}
