//! Record admitted constituent GPU steps using the same kernels as worker execution.
use super::*;

pub(super) fn record(
    original: &value::Value,
    calculation: &value::Calculation,
    region: Rect,
    fragment: &mut Fragment,
    planning: Planning<'_>,
) -> Result<Option<PlannedImage>, Error> {
    if value::constituents(calculation)?.is_none() {
        return Ok(None);
    }
    let budget = Budget {
        tile: planning.tile,
        scratch_bytes: planning.scratch_bytes,
    };
    let candidate = value::compile_unit(calculation, region, budget)?;
    let candidate = if let Some(limits) = planning.limits {
        candidate.admit_for(budget, limits)
    } else {
        candidate.admit(budget)
    };
    let unit = match candidate {
        Ok(unit) => unit,
        Err(Error::WorkingAllowance(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    if unit.output != region || unit.domain != original.domain() {
        return Ok(None);
    }
    let UnitWork::Gpu(work) = unit.work else {
        return Ok(None);
    };
    let output = value::image_resource(
        fragment,
        &original.request(region)?,
        region,
        planning.scope.is_some() && value::pixel_kernel(calculation),
    );
    let state = work
        .state
        .into_iter()
        .map(|bytes| fragment.resource(Storage::Buffer(bytes)))
        .collect::<Vec<_>>();
    for phase in work.phases {
        let phase = phase?;
        let compiled =
            (phase.kernel.as_ref() as &dyn std::any::Any).downcast_ref::<value::CompiledKernel>();
        let mut reads = phase
            .reads
            .iter()
            .enumerate()
            .map(|(index, read)| {
                let input = unit
                    .inputs
                    .get(read.input)
                    .ok_or(Error::InvalidParameter("read references an unknown input"))?;
                let mut source =
                    Fragment::with_programs(planning.scope.map(|scope| scope.programs().clone()));
                let callable = planning.policy == Policy::Composed
                    && compiled.is_some_and(|kernel| {
                        phase.inputs.iter().enumerate().all(|(slot, value)| {
                            !matches!(value, UnitValue::Read(read) if *read == index)
                                || kernel.can_sample(slot)
                        })
                    });
                let sample = if callable {
                    SampleInput::Callable(sample_region(input, read.region, &mut source, planning)?)
                } else {
                    SampleInput::Raster(evaluate_region(input, read.region, &mut source, planning)?)
                };
                let mut resources = Vec::new();
                // The source is complete before it joins the unit's not-yet-written state.
                // Its own admission must not inspect those unfinished resources.
                for storage in &source.resources {
                    resources.push(fragment.resource(storage.clone()));
                }
                let result = match sample {
                    SampleInput::Raster(image) => SampleInput::Raster(PlannedImage {
                        resource: resources[image.resource.0],
                        ..image
                    }),
                    SampleInput::Callable(mut sample) => {
                        for (_, binding) in &mut sample.bindings {
                            remap_binding(binding, &resources);
                        }
                        SampleInput::Callable(sample)
                    }
                };
                append_mapped(fragment, source.work, resources)?;
                Ok(result)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        if let Some(kernel) = compiled
            && reads
                .iter()
                .any(|read| matches!(read, SampleInput::Callable(_)))
        {
            match lower_callable_step(
                fragment,
                kernel,
                &phase.inputs,
                &reads,
                &state,
                output,
                planning,
            ) {
                Ok(()) => continue,
                Err(Error::WorkingAllowance(_)) => {
                    for (sample, read) in reads.iter_mut().zip(&phase.reads) {
                        if let SampleInput::Callable(callable) = sample {
                            let input = &unit.inputs[read.input];
                            let image = fragment.image(input.domain(), read.region);
                            callable.clone().materialize(fragment, image)?;
                            *sample = SampleInput::Raster(image);
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }
        let bindings = phase
            .inputs
            .iter()
            .map(|input| {
                match input {
                    UnitValue::Read(index) => reads.get(*index).and_then(|image| match image {
                        SampleInput::Raster(image) => Some(image.resource),
                        _ => None,
                    }),
                    UnitValue::State(index) => state.get(*index).copied(),
                    UnitValue::Output => Some(output.resource),
                }
                .ok_or(Error::InvalidParameter("unknown GPU phase binding"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Isolated linkage keeps shader code independent of preceding phase count.
        // Only resource references change when the recorded steps are composed.
        let mut step =
            Fragment::with_programs(planning.scope.map(|scope| scope.programs().clone()));
        let slots = bindings
            .iter()
            .enumerate()
            .map(|(slot, resource)| {
                step.resource(Storage::Input {
                    slot,
                    bytes: fragment.resources[resource.0].bytes(),
                    writable: !matches!(phase.inputs[slot], UnitValue::Read(_)),
                })
            })
            .collect::<Vec<_>>();
        let result = phase.kernel.lower(&mut step, &slots)?;
        if let Some(slot) = slots.iter().position(|slot| *slot == result.resource)
            && bindings[slot] == output.resource
            && (result.domain != output.domain || result.region != output.region)
        {
            return Err(Error::InvalidParameter(
                "GPU phase changed its result layout",
            ));
        }
        append(fragment, step, &bindings)?;
    }
    Ok(Some(output))
}

fn lower_callable_step(
    fragment: &mut Fragment,
    kernel: &value::CompiledKernel,
    inputs: &[UnitValue],
    reads: &[SampleInput],
    state: &[Resource],
    output: PlannedImage,
    planning: Planning<'_>,
) -> Result<(), Error> {
    let mut step = Fragment::with_programs(planning.scope.map(|scope| scope.programs().clone()));
    let mut aliases = Vec::new();
    let mut alias = |step: &mut Fragment, resource: Resource, writable| {
        let storage = &fragment.resources[resource.0];
        let local = step.resource(if matches!(storage, Storage::Uniform(_)) {
            storage.clone()
        } else {
            Storage::Input {
                slot: aliases.len(),
                bytes: storage.bytes(),
                writable,
            }
        });
        if local.0 == aliases.len() {
            aliases.push(resource);
        }
        local
    };
    let state_resource = |value: &UnitValue| match value {
        UnitValue::State(index) => state.get(*index).copied(),
        UnitValue::Output => Some(output.resource),
        UnitValue::Read(_) => None,
    };
    let mut samples = Vec::new();
    for (index, input) in inputs.iter().take(kernel.inputs.len()).enumerate() {
        let sample = match input {
            UnitValue::Read(read) => match reads
                .get(*read)
                .ok_or(Error::InvalidParameter("unknown GPU phase read"))?
                .clone()
            {
                SampleInput::Raster(image) => SampleInput::Raster(PlannedImage {
                    resource: alias(&mut step, image.resource, false),
                    ..image
                }),
                SampleInput::Callable(mut sample) => {
                    for (_, binding) in &mut sample.bindings {
                        if let Some(resource) = binding.local() {
                            let local = alias(&mut step, resource, false);
                            match binding {
                                Binding::Read(r)
                                | Binding::Write(r)
                                | Binding::Uniform(r)
                                | Binding::Texture(r) => *r = local,
                                _ => unreachable!(),
                            }
                        }
                    }
                    SampleInput::Callable(sample)
                }
            },
            _ => SampleInput::Raster(PlannedImage {
                resource: alias(
                    &mut step,
                    state_resource(input)
                        .ok_or(Error::InvalidParameter("unknown GPU phase state"))?,
                    true,
                ),
                ..kernel.inputs[index]
            }),
        };
        samples.push(sample);
    }
    let target = inputs
        .last()
        .and_then(state_resource)
        .ok_or(Error::InvalidParameter("unknown GPU phase output"))?;
    let resource = alias(&mut step, target, true);
    let result = kernel.lower_samples(&mut step, samples, resource)?;
    if target == output.resource
        && (result.domain != output.domain || result.region != output.region)
    {
        return Err(Error::InvalidParameter(
            "GPU phase changed its result layout",
        ));
    }
    let baseline = wgpu::Limits::default();
    let limits = planning.limits.unwrap_or(&baseline);
    if !step.work.iter().all(|work| bindings_fit(work, limits)) {
        return Err(Error::WorkingAllowance(
            "composed phase exceeds device bindings",
        ));
    }
    append(fragment, step, &aliases)
}

fn remap_binding(binding: &mut Binding, resources: &[Resource]) {
    match binding {
        Binding::Read(resource)
        | Binding::Write(resource)
        | Binding::Uniform(resource)
        | Binding::Texture(resource) => *resource = resources[resource.0],
        Binding::Shared(_) => {}
    }
}

fn append(target: &mut Fragment, step: Fragment, aliases: &[Resource]) -> Result<(), Error> {
    let mut resources = aliases.to_vec();
    for (index, storage) in step.resources.into_iter().enumerate() {
        if index < aliases.len() {
            if !matches!(storage, Storage::Input { slot, .. } if slot == index)
                && !(matches!(storage, Storage::Uniform(_))
                    && storage == target.resources[aliases[index].0])
            {
                return Err(Error::InvalidParameter(
                    "GPU phase replaced its declared storage",
                ));
            }
        } else {
            resources.push(target.resource(storage));
        }
    }
    append_mapped(target, step.work, resources)
}

fn append_mapped(
    target: &mut Fragment,
    work: Vec<Work>,
    resources: Vec<Resource>,
) -> Result<(), Error> {
    let remap = |binding: &mut Binding| remap_binding(binding, &resources);
    for mut work in work {
        match &mut work {
            Work::Produce { output, .. } | Work::Upload { output, .. } => {
                output.resource = resources[output.resource.0]
            }
            Work::Copy { source, target, .. } => {
                *source = resources[source.0];
                *target = resources[target.0];
            }
            Work::Compute {
                bindings,
                initialize,
                ..
            } => {
                for (_, binding) in bindings {
                    remap(binding);
                }
                if let Some(resource) = initialize {
                    *resource = resources[resource.0];
                }
            }
            Work::Render {
                bindings, target, ..
            } => {
                for (_, binding) in bindings {
                    remap(binding);
                }
                *target = resources[target.0];
            }
        }
        target.work.push(work);
    }
    Ok(())
}
