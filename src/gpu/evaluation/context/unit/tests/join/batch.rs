use super::*;

struct StatefulCopy {
    clears: Arc<AtomicUsize>,
}
impl StatefulCopy {
    fn unit(&self, node: &Value) -> WorkUnit {
        let inputs = node.inputs();
        let domain = node.domain();
        let output = domain.finite().unwrap();
        WorkUnit {
            inputs: inputs.to_vec(),
            domain,
            output,
            work: UnitWork::Gpu(GpuUnit {
                state: vec![u64::from(output.width) * u64::from(output.height) * 16],
                phases: Box::new(
                    vec![
                        GpuPhase {
                            reads: vec![],
                            inputs: vec![UnitValue::State(0)],
                            kernel: Arc::new(ClearGpu {
                                domain,
                                region: output,
                                clears: self.clears.clone(),
                            }),
                        },
                        GpuPhase {
                            reads: vec![Read::new(0, output)],
                            inputs: vec![UnitValue::Read(0), UnitValue::State(0)],
                            kernel: Arc::new(CopyGpu {
                                domain,
                                region: output,
                            }),
                        },
                        GpuPhase {
                            reads: vec![],
                            inputs: vec![UnitValue::State(0), UnitValue::Output],
                            kernel: Arc::new(CopyGpu {
                                domain,
                                region: output,
                            }),
                        },
                    ]
                    .into_iter()
                    .map(Ok),
                ),
            }),
        }
    }
}

fn retire_batch(independent_consumer: bool) {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let gate = Arc::new(Gate::default());
    let clears = Arc::new(AtomicUsize::new(0));
    let payloads = [Arc::new(vec![0]), Arc::new(vec![1])];
    let weak = payloads.each_ref().map(Arc::downgrade);
    let roots = payloads.map(|payload| {
        let input = root(gate.clone(), payload);
        backend_value(input)
    });
    let identities = roots
        .each_ref()
        .map(|root| root.identity().unwrap().clone());
    let budget = Budget {
        tile: [8, 4],
        scratch_bytes: 1 << 20,
    };
    let evaluations = roots.each_ref().map(|root| {
        context
            .request(
                root.clone(),
                Region {
                    bounds: root.domain().finite().unwrap(),
                    grid: root.domain().grid,
                },
                budget,
            )
            .unwrap()
    });
    let requests = evaluations
        .iter()
        .map(|evaluation| {
            let image = evaluation.job.image();
            let layout = ImageLayout {
                domain: image.domain(),
                region: evaluation.region().bounds,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            };
            (
                image
                    .output(layout.region)
                    .unwrap()
                    .identity()
                    .unwrap()
                    .clone(),
                Arc::new(layout),
                layout.bytes(),
                evaluation.request_policy(),
            )
        })
        .collect();
    let jobs = evaluations
        .iter()
        .map(|evaluation| {
            (
                evaluation.clone(),
                Some(
                    StatefulCopy {
                        clears: clears.clone(),
                    }
                    .unit(evaluation.job.image()),
                ),
            )
        })
        .collect::<Vec<_>>();
    let (owned_context, owned_queue, owned_scope) = (context.clone(), queue.clone(), scope.clone());
    let mut futures = scope.obtain_many_for(requests, move |missing| async move {
        crate::background::run(move |stop| {
            let gate = crate::gpu::submission::SubmissionGate::default();
            let working = HostWorkingSet::default();
            let cancelled = || stop.is_cancelled();
            let worker = Worker {
                context: &owned_context,
                queue: &owned_queue,
                gate: &gate,
                scope: &owned_scope,
                budget,
                working: &working,
                cancelled: &cancelled,
                progress: &|| {},
            };
            let mut selected = jobs
                .into_iter()
                .enumerate()
                .filter(|(index, _)| missing.contains(index))
                .map(|(_, job)| job)
                .collect::<Vec<_>>();
            let mut total = 0;
            let targets = selected
                .iter_mut()
                .map(|(evaluation, unit)| {
                    let node = evaluation.job.image();
                    let region = evaluation.region().bounds;
                    let unit = unit
                        .take()
                        .unwrap()
                        .admit_for(budget, &owned_context.device().limits())?;
                    total += u64::from(region.width) * u64::from(region.height) * 32;
                    Ok(super::super::super::super::batch_units::Target {
                        evaluation,
                        layout: ImageLayout {
                            domain: node.domain(),
                            region,
                            format: ImageFormat::Rgba32Float,
                            backing: ImageBacking::Buffer,
                        },
                        unit: Some(unit),
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            worker
                .materialize_targets(targets, total, budget.scratch_bytes)?
                .ok_or(Error::Cancelled)
        })
        .await
        .map_err(|_| Error::Gpu("backend batch worker panicked".into()))?
    });
    let mut first = Box::pin(futures.remove(0));
    let second = futures.remove(0);
    assert!(futures_lite::future::block_on(futures_lite::future::poll_once(&mut first)).is_none());
    until(|| gate.entered.load(Ordering::Acquire) == 1);
    assert_eq!(
        clears.load(Ordering::Acquire),
        2,
        "both units must have entered the common phase batch before its first CPU input blocks"
    );
    let independent = if independent_consumer {
        let mut future = Box::pin(
            context
                .materialize(
                    &queue,
                    &Default::default(),
                    &scope,
                    evaluations[1].clone(),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        );
        assert!(
            futures_lite::future::block_on(futures_lite::future::poll_once(&mut future)).is_none()
        );
        until(|| scope.pending_consumers::<ImageLayout>(&identities[1], |_| true) >= 2);
        Some(future)
    } else {
        None
    };
    drop(evaluations);
    drop(roots);
    drop(first);
    drop(second);
    if let Some(independent) = independent {
        gate.release.store(true, Ordering::Release);
        let image = pollster::block_on(independent).unwrap();
        let expected = (0..4)
            .flat_map(|y| (0..8).map(move |x| [2. * x as f32 + 1., 2. * y as f32, 0., 0.]))
            .collect::<Vec<_>>();
        assert_eq!(
            read_buffer(&device, &queue, image.as_buffer().unwrap()),
            expected
        );
        assert!(
            !gate.cancelled.load(Ordering::Acquire),
            "retiring the entire batch cannot stop an independent joined root consumer"
        );
        assert_eq!(gate.entered.load(Ordering::Acquire), 2);
    } else {
        until(|| gate.cancelled.load(Ordering::Acquire));
        assert!(
            weak.iter().all(|payload| payload.upgrade().is_some()),
            "all captured inputs survive physical cleanup"
        );
        assert!(
            identities
                .iter()
                .all(|identity| scope.ready::<ImageLayout>(identity).is_empty()),
            "no root output can be published while its input is cancelled"
        );
        gate.cleanup.store(true, Ordering::Release);
        until(|| gate.returned.load(Ordering::Acquire));
        assert_eq!(
            gate.entered.load(Ordering::Acquire),
            1,
            "retirement must not start the remaining CPU input"
        );
    }
    until(|| weak.iter().all(|payload| payload.upgrade().is_none()));
    assert_eq!(clears.load(Ordering::Acquire), 2);
}

#[test]
fn retiring_a_running_phase_batch_preserves_an_independent_joined_root() {
    retire_batch(true);
}

#[test]
fn retiring_the_last_phase_batch_consumers_stops_inputs_and_retains_captures_through_cleanup() {
    retire_batch(false);
}
