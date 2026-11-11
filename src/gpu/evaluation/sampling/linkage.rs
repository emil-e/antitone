//! Private shader-library namespaces, resource linkage, and export reachability.
use super::*;

impl Sampled {
    /// Link a callable image into a sibling namespace without materializing its pixels.
    pub(crate) fn namespaced(
        self,
        fragment: &Fragment,
        prefix: &str,
        next_slot: &mut u32,
    ) -> Result<Self, Error> {
        let (code, bindings) = link(
            fragment,
            &self.code,
            &self.bindings,
            &[&self.symbol],
            prefix,
            next_slot,
        )?;
        Ok(Self {
            code,
            symbol: format!("{prefix}{}", self.symbol),
            bindings,
        })
    }
}

#[derive(Hash, PartialEq, Eq)]
struct Key {
    prefix: String,
    symbols: Vec<String>,
    first_slot: u32,
    captures: Vec<(u32, usize)>,
}
struct Linked {
    code: Arc<str>,
    bindings: Vec<(u32, usize)>,
    next_slot: u32,
}
type Entry = std::sync::OnceLock<Result<Arc<Linked>, Error>>;
#[derive(Default)]
pub(super) struct Links(std::sync::Mutex<std::collections::HashMap<Key, Arc<Entry>>>);

type BoundCode = (Arc<str>, Vec<(u32, Binding)>);

fn link(
    fragment: &Fragment,
    code: &str,
    captured: &[(u32, Binding)],
    symbols: &[&str],
    prefix: &str,
    next_slot: &mut u32,
) -> Result<BoundCode, Error> {
    let library = fragment.library(code)?;
    let key = Key {
        prefix: prefix.into(),
        symbols: symbols.iter().map(|symbol| (*symbol).into()).collect(),
        first_slot: *next_slot,
        captures: captured
            .iter()
            .enumerate()
            .map(|(index, (slot, binding))| {
                (
                    *slot,
                    captured[..index]
                        .iter()
                        .position(|(_, prior)| prior == binding)
                        .unwrap_or(index),
                )
            })
            .collect(),
    };
    let entry = library
        .links
        .0
        .lock()
        .unwrap()
        .entry(key)
        .or_default()
        .clone();
    let linked = entry
        .get_or_init(|| build(&library, captured, symbols, prefix, *next_slot).map(Arc::new))
        .as_ref()
        .map_err(Clone::clone)?;
    *next_slot = linked.next_slot;
    Ok((
        linked.code.clone(),
        linked
            .bindings
            .iter()
            .map(|(slot, index)| (*slot, captured[*index].1.clone()))
            .collect(),
    ))
}

fn build(
    library: &Library,
    captured: &[(u32, Binding)],
    symbols: &[&str],
    prefix: &str,
    mut next: u32,
) -> Result<Linked, Error> {
    let next_slot = &mut next;
    let mut module = library.module.clone();
    for (slot, _) in captured {
        if !module.global_variables.iter().any(|(_, v)| {
            v.binding
                .as_ref()
                .is_some_and(|b| b.group == 0 && b.binding == *slot)
        }) {
            return Err(Error::InvalidParameter(
                "pointwise capture has no shader resource",
            ));
        }
    }
    let used = library.resources(symbols)?;
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
    let mut bindings = Vec::new();
    for (_, value) in module.global_variables.iter_mut() {
        rename(&mut value.name, prefix);
        if let Some(binding) = &mut value.binding {
            if binding.group != 0 {
                return Err(Error::InvalidParameter(
                    "pointwise resources require group zero",
                ));
            }
            let slot = *next_slot;
            *next_slot += 1;
            if used.contains(&binding.binding) {
                let resource = captured.iter().find(|(s, _)| *s == binding.binding).ok_or(
                    Error::InvalidParameter("pointwise calculation lacks a captured resource"),
                )?;
                bindings.push((slot, resource.1.clone()));
            }
            binding.binding = slot;
        }
    }
    super::library::share_uniforms(&mut module, &mut bindings);
    let exports = symbols
        .iter()
        .map(|symbol| format!("{prefix}{symbol}"))
        .collect::<Vec<_>>();
    super::library::compact(
        &mut module,
        &exports.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
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
    Ok(Linked {
        code: code.into(),
        bindings: bindings
            .into_iter()
            .map(|(slot, binding)| {
                (
                    slot,
                    captured
                        .iter()
                        .position(|(_, capture)| *capture == binding)
                        .unwrap(),
                )
            })
            .collect(),
        next_slot: next,
    })
}

fn rename(name: &mut Option<String>, prefix: &str) {
    if let Some(name) = name {
        *name = format!("{prefix}{name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CODE: &str = "@group(0) @binding(0) var<uniform> a:vec4f; @group(0) @binding(1) var<uniform> b:vec4f; fn first()->f32{return a.x;} fn both()->f32{return a.x+b.y;}";
    #[test]
    fn concurrent_linking_shares_only_code_and_rebinds_each_consumers_resources() {
        let programs = Arc::new(Programs::default());
        let ready = Arc::new(std::sync::Barrier::new(2));
        let threads = [10, 30].map(|offset| {
            let programs = programs.clone();
            let ready = ready.clone();
            std::thread::spawn(move || {
                let fragment = Fragment::with_programs(Some(programs));
                let captures = [
                    (0, Binding::Uniform(Resource(offset))),
                    (1, Binding::Uniform(Resource(offset + 1))),
                ];
                ready.wait();
                let (code, bindings) =
                    link(&fragment, CODE, &captures, &["both"], "unit_", &mut 64).unwrap();
                assert_eq!(
                    bindings
                        .iter()
                        .map(|(_, binding)| binding.local().unwrap().0)
                        .collect::<Vec<_>>(),
                    [offset, offset + 1]
                );
                code
            })
        });
        let [first, second] = threads.map(|thread| thread.join().unwrap());
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn linked_templates_rebind_current_resources_and_distinguish_aliases_and_exports() {
        let fragment = Fragment::with_programs(Some(Arc::default()));
        let captures = |a, b| {
            vec![
                (0, Binding::Uniform(Resource(a))),
                (1, Binding::Uniform(Resource(b))),
            ]
        };
        let apply = |a, b, export, prefix, start| {
            let captured = captures(a, b);
            let mut slot = start;
            let (code, bindings) =
                link(&fragment, CODE, &captured, &[export], prefix, &mut slot).unwrap();
            (code, bindings, slot)
        };
        let (first, bindings, end) = apply(0, 1, "both", "unit_", 64);
        assert_eq!(
            bindings
                .iter()
                .map(|(_, binding)| binding.local().unwrap().0)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(end, 66);
        let (hot, bindings, end) = apply(17, 29, "both", "unit_", 64);
        assert!(
            Arc::ptr_eq(&first, &hot),
            "reuse must consume the linked template, not merely regenerate equal code"
        );
        assert_eq!(
            bindings
                .iter()
                .map(|(_, binding)| binding.local().unwrap().0)
                .collect::<Vec<_>>(),
            [17, 29]
        );
        assert_eq!(end, 66);
        let (shared, bindings, _) = apply(7, 7, "both", "unit_", 64);
        assert!(!Arc::ptr_eq(&first, &shared));
        assert_eq!(bindings.len(), 1);
        assert!(matches!(bindings[0].1, Binding::Uniform(Resource(7))));
        let (_, bindings, _) = apply(17, 29, "first", "unit_", 64);
        assert_eq!(bindings.len(), 1);
        assert!(matches!(bindings[0].1, Binding::Uniform(Resource(17))));
        let (other, bindings, end) = apply(17, 29, "both", "other_", 64);
        assert!(other.contains("other_both"));
        assert_eq!(
            bindings.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
            [64, 65]
        );
        assert_eq!(end, 66);
        let (shifted, bindings, end) = apply(17, 29, "both", "unit_", 80);
        assert!(shifted.contains("unit_both"));
        assert_eq!(
            bindings.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
            [80, 81]
        );
        assert_eq!(end, 82);
        let mut slot = 64;
        assert!(
            link(
                &fragment,
                CODE,
                &captures(0, 1),
                &["missing"],
                "unit_",
                &mut slot
            )
            .is_err()
        );
        let (again, _, _) = apply(31, 47, "both", "unit_", 64);
        assert!(Arc::ptr_eq(&first, &again));
    }
}
