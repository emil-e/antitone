//! Keep a linked library's requested exports and their transitive shader dependencies.
use super::*;

pub(super) fn share_uniforms(module: &mut naga::Module, bindings: &mut Vec<(u32, Binding)>) {
    let mut canonical = Vec::<(
        naga::Handle<naga::GlobalVariable>,
        naga::Handle<naga::Type>,
        Binding,
    )>::new();
    let mut aliases = std::collections::HashMap::new();
    let mut retired = Vec::new();
    for (handle, global) in module.global_variables.iter() {
        if global.space != naga::AddressSpace::Uniform {
            continue;
        }
        let Some(binding) = global
            .binding
            .as_ref()
            .and_then(|slot| bindings.iter().find(|(index, _)| *index == slot.binding))
        else {
            continue;
        };
        if let Some((other, _, _)) = canonical
            .iter()
            .find(|(_, ty, value)| *ty == global.ty && value == &binding.1)
        {
            aliases.insert(handle, *other);
            retired.push(binding.0);
        } else {
            canonical.push((handle, global.ty, binding.1.clone()));
        }
    }
    let remap = |expression: &mut naga::Expression| {
        if let naga::Expression::GlobalVariable(handle) = expression
            && let Some(other) = aliases.get(handle)
        {
            *handle = *other;
        }
    };
    for (_, expression) in module.global_expressions.iter_mut() {
        remap(expression);
    }
    for (_, function) in module.functions.iter_mut() {
        for (_, expression) in function.expressions.iter_mut() {
            remap(expression);
        }
    }
    for entry in &mut module.entry_points {
        for (_, expression) in entry.function.expressions.iter_mut() {
            remap(expression);
        }
    }
    bindings.retain(|(slot, _)| !retired.contains(slot));
}

pub(super) fn compact(module: &mut naga::Module, symbols: &[&str]) -> Result<(), Error> {
    let mut function = naga::Function::default();
    for symbol in symbols {
        let (handle, target) = module
            .functions
            .iter()
            .find(|(_, target)| target.name.as_deref() == Some(*symbol))
            .ok_or(Error::InvalidParameter("shader library export is absent"))?;
        let arguments = target
            .arguments
            .iter()
            .map(|argument| {
                function.expressions.append(
                    naga::Expression::ZeroValue(argument.ty),
                    naga::Span::UNDEFINED,
                )
            })
            .collect();
        let result = target.result.as_ref().map(|_| {
            function
                .expressions
                .append(naga::Expression::CallResult(handle), naga::Span::UNDEFINED)
        });
        function.body.push(
            naga::Statement::Call {
                function: handle,
                arguments,
                result,
            },
            naga::Span::UNDEFINED,
        );
    }
    // Naga's compactor roots reachability at entry points. This entry is never emitted or run.
    module.entry_points.push(naga::EntryPoint {
        name: "library_exports".into(),
        stage: naga::ShaderStage::Compute,
        early_depth_test: None,
        workgroup_size: [1; 3],
        workgroup_size_overrides: None,
        function,
        mesh_info: None,
        task_payload: None,
        incoming_ray_payload: None,
    });
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module)
    .map_err(|error| Error::Gpu(format!("shader library exports: {error:?}")))?;
    naga::compact::compact(module, naga::compact::KeepUnused::No);
    module.entry_points.clear();
    Ok(())
}
