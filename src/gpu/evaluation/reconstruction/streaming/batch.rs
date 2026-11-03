use super::*;

pub(in crate::gpu::evaluation::reconstruction) fn produce(
    outputs: &[(&Arc<ImageNode>, Production<'_>)],
) -> Result<bool, Error> {
    let Some((first, _)) = outputs.first() else {
        return Ok(false);
    };
    let Some(input) = first.input.as_ref() else {
        return Ok(false);
    };
    let mut operations = Vec::new();
    for (image, worker) in outputs {
        let Some(operation) =
            (image.operation.as_ref() as &dyn std::any::Any).downcast_ref::<Reconstruction>()
        else {
            return Ok(false);
        };
        let Some(upstream) = image.input.as_ref() else {
            return Ok(false);
        };
        if !(Arc::ptr_eq(input, upstream)
            || input.identity().is_some() && input.identity() == upstream.identity())
            || worker.budget.tile[0] < worker.layout.region.width
            || worker.budget.tile[1] < worker.layout.region.height
        {
            return Ok(false);
        }
        operations.push(operation);
    }
    if outputs.len() < 2 {
        return Ok(false);
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
    if resident {
        for ((_, worker), plan) in outputs.iter().zip(plans) {
            worker.deliver(&mut executor, plan)?;
        }
    } else {
        drop(plans);
        let targets: Vec<_> = operations
            .iter()
            .zip(outputs)
            .map(|(op, (_, worker))| (*op, worker, worker.layout.region))
            .collect();
        stream(&targets, input, &mut executor)?;
    }
    Ok(true)
}

struct Reduction<'a, 'b> {
    operation: &'a Reconstruction,
    worker: &'a Production<'b>,
    region: Rect,
    means: wgpu::Buffer,
    flags: wgpu::Buffer,
}
impl Reduction<'_, '_> {
    fn accumulate(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: PlannedImage,
        pixels: &wgpu::Buffer,
        affected: Rect,
    ) -> Result<(), Error> {
        let mut fragment = Fragment::default();
        let part = source.region;
        let source = PlannedImage {
            resource: fragment.resource(Storage::Input {
                slot: 0,
                bytes: pixels.size(),
                writable: false,
            }),
            ..source
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
        self.worker.context.encode_work(
            fragment,
            result,
            encoder,
            &[pixels, &self.means, &self.flags],
        )
    }
}

pub(super) fn stream(
    outputs: &[(&Reconstruction, &Production<'_>, Rect)],
    input: &Arc<ImageNode>,
    executor: &mut Executor,
) -> Result<(), Error> {
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
    let available = budget
        .checked_sub(scratch + 1024 * outputs.len() as u64)
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
    let buffer = |bytes| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("required reconstruction reduction"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    };
    let targets: Vec<_> = outputs
        .iter()
        .map(|(operation, worker, region)| Reduction {
            operation,
            worker,
            region: *region,
            means: buffer(u64::from(region.width) * u64::from(region.height) * 16),
            flags: buffer(u64::from(region.width) * u64::from(region.height) * 4),
        })
        .collect();
    for target in &targets {
        target
            .operation
            .clear(target.worker, target.region, &target.means, &target.flags)?;
        (worker.progress)();
    }
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
                let mut plan = super::super::super::plan::probe(
                    input,
                    part,
                    Budget {
                        tile: [part.width, part.height],
                        scratch_bytes: available,
                    },
                    worker.scope,
                    &limits,
                )?;
                let admission = Executor::admit(&limits, &plan);
                if plan.peak_scratch_bytes > available || admission.is_err() {
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
                plan.scratch_budget = available;
                worker
                    .run_sink(executor, &plan, |encoder, source, pixels| {
                        for (target, region) in &affected {
                            target.accumulate(encoder, source, pixels, *region)?;
                        }
                        Ok(())
                    })?
                    .ok_or(Error::Cancelled)?;
                (worker.progress)();
            }
        }
    }
    for target in targets {
        target
            .operation
            .finish(target.worker, target.region, &target.means, &target.flags)?;
    }
    Ok(())
}
