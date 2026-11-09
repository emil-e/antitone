//! The same authored pixel function can consume a raster or a callable image.
use super::*;
#[cfg(test)]
mod tests;

impl Sampled {
    pub(crate) fn map(
        self,
        fragment: &mut Fragment,
        domain: Domain,
        region: Rect,
        functions: Vec<Function>,
    ) -> Result<Self, Error> {
        let mut code = self.code.to_string();
        let input_module = parse(&code)?;
        let input_usage = resources(&input_module, &[self.symbol.as_str()])?;
        let mut bindings = self
            .bindings
            .into_iter()
            .filter(|(slot, _)| input_usage.contains(slot))
            .collect::<Vec<_>>();
        let mut calls = String::new();
        // Linked libraries use their own resource namespace, outside the caller ABIs.
        let mut slot = input_module
            .global_variables
            .iter()
            .filter_map(|(_, v)| v.binding.as_ref().map(|b| b.binding))
            .max()
            .unwrap_or(63)
            .max(63)
            + 1;
        let mut prefix = format!("linked_{}_", code.len());
        while code.contains(&prefix) {
            prefix.push('_');
        }
        let mut libraries: Vec<(Function, Vec<&'static str>)> = vec![];
        let mut order = vec![];
        for function in functions {
            let compatible = libraries.iter().position(|(library, _)| {
                library.code == function.code
                    && function.bindings.iter().all(|(slot, value)| {
                        library
                            .bindings
                            .iter()
                            .all(|(s, v)| s != slot || v == value)
                    })
            });
            let index = compatible.unwrap_or(libraries.len());
            let symbol = function.symbol;
            if let Some((library, symbols)) = libraries.get_mut(index) {
                for (slot, resource) in function.bindings {
                    if !library.bindings.iter().any(|(s, _)| *s == slot) {
                        library.bindings.push((slot, resource));
                    }
                }
                symbols.push(symbol);
            } else {
                libraries.push((function, vec![symbol]));
            }
            order.push((index, symbol));
        }
        for (i, (library, symbols)) in libraries.into_iter().enumerate() {
            let namespace = format!("{prefix}{i}_");
            let (library, resources) = link(library, &symbols, &namespace, &mut slot, fragment)?;
            code.push_str(&library);
            code.push('\n');
            bindings.extend(resources);
        }
        for (index, symbol) in order {
            calls.push_str(&format!(
                "pixel = {prefix}{index}_{symbol}(pixel, position);\n"
            ));
        }
        let origin = domain.point([f64::from(region.x) + 0.5, f64::from(region.y) + 0.5]);
        let geometry = fragment.uniform(&[
            [
                origin[0] as f32,
                origin[1] as f32,
                region.x as f32,
                region.y as f32,
            ],
            [
                domain.grid.axes[0][0],
                domain.grid.axes[0][1],
                domain.grid.axes[1][0],
                domain.grid.axes[1][1],
            ]
            .map(|v| v as f32),
        ]);
        let symbol = format!("{prefix}sample");
        code.push_str(&format!(
            r#"
@group(0) @binding({slot}) var<uniform> {prefix}geometry: array<vec4f, 2>;
fn {symbol}(p: vec2i) -> vec4f {{
    var pixel = {}(p);
    if pixel.w < 0.0 {{ return pixel; }}
    let local = vec2f(p) - {prefix}geometry[0].zw;
    let position = {prefix}geometry[0].xy + local.x * {prefix}geometry[1].xy + local.y * {prefix}geometry[1].zw;
    {calls}
    return pixel;
}}
"#,
            self.symbol,
        ));
        bindings.push((slot, geometry));
        Ok(Self {
            code: code.into(),
            symbol,
            bindings,
        })
    }
}

fn parse(code: &str) -> Result<naga::Module, Error> {
    naga::front::wgsl::parse_str(code)
        .map_err(|error| Error::Gpu(format!("pointwise WGSL: {error}")))
}

fn resources(module: &naga::Module, symbols: &[&str]) -> Result<Vec<u32>, Error> {
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module)
    .map_err(|error| Error::Gpu(format!("pointwise library validation: {error}")))?;
    let roots = symbols
        .iter()
        .map(|symbol| {
            module
                .functions
                .iter()
                .find(|(_, f)| f.name.as_deref() == Some(*symbol))
                .map(|(handle, _)| handle)
                .ok_or(Error::InvalidParameter(
                    "pointwise function is absent from its module",
                ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(module
        .global_variables
        .iter()
        .filter_map(|(handle, value)| {
            value
                .binding
                .as_ref()
                .filter(|_| roots.iter().any(|root| !info[*root][handle].is_empty()))
                .map(|binding| binding.binding)
        })
        .collect())
}

fn link(
    function: Function,
    symbols: &[&'static str],
    prefix: &str,
    next_slot: &mut u32,
    fragment: &mut Fragment,
) -> Result<(String, Vec<(u32, Binding)>), Error> {
    // Only shader structure is reusable. Captured values are lowered afresh for
    // every plan, including candidates with the same code but different uniforms.
    let cached = function.linked.lock().unwrap().clone().filter(|cached| {
        cached.source == function.code
            && cached.symbols == symbols
            && cached.prefix == prefix
            && cached.first_slot == *next_slot
    });
    let linked = match cached {
        Some(linked) => linked,
        None => {
            let linked = Arc::new(compile_link(&function, symbols, prefix, *next_slot)?);
            *function.linked.lock().unwrap() = Some(linked.clone());
            linked
        }
    };
    if function
        .bindings
        .iter()
        .any(|(slot, _)| !linked.declared.contains(slot))
    {
        return Err(Error::InvalidParameter(
            "pointwise capture has no shader resource",
        ));
    }
    let bindings = linked
        .captures
        .iter()
        .map(|&(source, target)| {
            let binding = function
                .bindings
                .iter()
                .find(|(slot, _)| *slot == source)
                .ok_or(Error::InvalidParameter(
                    "pointwise calculation lacks a captured resource",
                ))?;
            Ok((target, binding.1.lower(fragment)))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    *next_slot = linked.next_slot;
    Ok((linked.code.to_string(), bindings))
}

fn compile_link(
    function: &Function,
    symbols: &[&'static str],
    prefix: &str,
    first_slot: u32,
) -> Result<super::super::contract::FunctionLink, Error> {
    let mut module = parse(&function.code)?;
    module.entry_points.clear();
    let used = resources(&module, symbols)?;
    let types = module
        .types
        .iter()
        .filter_map(|(handle, ty)| {
            ty.name.as_ref().map(|name| {
                let mut ty = ty.clone();
                ty.name = Some(format!("{prefix}{name}"));
                (handle, ty)
            })
        })
        .collect::<Vec<_>>();
    for (handle, ty) in types {
        module.types.replace(handle, ty);
    }
    for (_, value) in module.constants.iter_mut() {
        rename(&mut value.name, prefix);
    }
    for (_, value) in module.overrides.iter_mut() {
        rename(&mut value.name, prefix);
    }
    for (_, value) in module.functions.iter_mut() {
        rename(&mut value.name, prefix);
    }
    let mut next_slot = first_slot;
    let mut declared = Vec::new();
    let mut captures = Vec::new();
    for (_, value) in module.global_variables.iter_mut() {
        rename(&mut value.name, prefix);
        if let Some(binding) = &mut value.binding {
            if binding.group != 0 {
                return Err(Error::InvalidParameter(
                    "pointwise resources require group zero",
                ));
            }
            declared.push(binding.binding);
            let slot = next_slot;
            next_slot += 1;
            if used.contains(&binding.binding) {
                captures.push((binding.binding, slot));
            }
            binding.binding = slot;
        }
    }
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|error| Error::Gpu(format!("pointwise library validation: {error}")))?;
    let code = naga::back::wgsl::write_string(
        &module,
        &info,
        naga::back::wgsl::WriterFlags::EXPLICIT_TYPES,
    )
    .map_err(|error| Error::Gpu(format!("pointwise library WGSL: {error}")))?;
    Ok(super::super::contract::FunctionLink {
        source: function.code.clone(),
        symbols: symbols.to_vec(),
        prefix: prefix.to_owned(),
        first_slot,
        next_slot,
        declared,
        captures,
        code: code.into(),
    })
}

fn rename(name: &mut Option<String>, prefix: &str) {
    if let Some(name) = name {
        *name = format!("{prefix}{name}");
    }
}
