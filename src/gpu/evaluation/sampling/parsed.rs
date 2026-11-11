//! Parsed shader libraries belong to the program compiler, independently of bound resources.
use super::*;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

type Entry = OnceLock<Result<Arc<Library>, Error>>;

#[derive(Default)]
pub(in crate::gpu::evaluation) struct Libraries(Mutex<HashMap<Arc<str>, Arc<Entry>>>);

impl Libraries {
    pub(in crate::gpu::evaluation) fn get(&self, code: &str) -> Result<Arc<Library>, Error> {
        let entry = {
            let mut libraries = self.0.lock().unwrap();
            if let Some(entry) = libraries.get(code) {
                entry.clone()
            } else {
                let entry = Arc::new(Entry::new());
                libraries.insert(code.into(), entry.clone());
                entry
            }
        };
        entry
            .get_or_init(|| Library::parse(code).map(Arc::new))
            .clone()
    }
}

pub(in crate::gpu::evaluation) struct Library {
    pub module: naga::Module,
    pub authored_entries: bool,
    pub(super) links: super::linkage::Links,
    info: OnceLock<Result<naga::valid::ModuleInfo, Error>>,
}
impl Library {
    pub(in crate::gpu::evaluation) fn parse(code: &str) -> Result<Self, Error> {
        #[cfg(test)]
        testing::parsed();
        let mut module = naga::front::wgsl::parse_str(code)
            .map_err(|error| Error::Gpu(format!("pointwise WGSL: {error}")))?;
        let authored_entries = !module.entry_points.is_empty();
        module.entry_points.clear();
        Ok(Self {
            module,
            authored_entries,
            links: Default::default(),
            info: OnceLock::new(),
        })
    }

    fn info(&self) -> Result<&naga::valid::ModuleInfo, Error> {
        self.info
            .get_or_init(|| {
                #[cfg(test)]
                testing::validated();
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&self.module)
                .map_err(|error| Error::Gpu(format!("pointwise library validation: {error}")))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    // ABI arguments may be unused; retain coordinates whenever the body references them,
    // including calls, rather than spending a binding on an unobserved value.
    pub(in crate::gpu::evaluation) fn argument_used(
        &self,
        symbol: &str,
        argument: u32,
    ) -> Result<bool, Error> {
        let (handle, function) = self
            .module
            .functions
            .iter()
            .find(|(_, function)| function.name.as_deref() == Some(symbol))
            .ok_or(Error::InvalidParameter(
                "pointwise function is absent from its module",
            ))?;
        let info = &self.info()?[handle];
        Ok(function.expressions.iter().any(|(handle, expression)| {
            matches!(expression, naga::Expression::FunctionArgument(index) if *index == argument)
                && info[handle].ref_count != 0
        }))
    }

    pub(in crate::gpu::evaluation) fn resources(
        &self,
        symbols: &[&str],
    ) -> Result<Vec<u32>, Error> {
        let info = self.info()?;
        let roots = symbols
            .iter()
            .map(|symbol| {
                self.module
                    .functions
                    .iter()
                    .find(|(_, function)| function.name.as_deref() == Some(*symbol))
                    .map(|(handle, _)| handle)
                    .ok_or(Error::InvalidParameter(
                        "pointwise function is absent from its module",
                    ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self
            .module
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
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(in crate::gpu::evaluation) mod testing {
    use std::cell::Cell;
    thread_local! { static COUNTS: Cell<Option<(usize, usize)>> = const { Cell::new(None) }; }
    pub(super) fn parsed() {
        COUNTS.update(|value| value.map(|(a, b)| (a + 1, b)));
    }
    pub(super) fn validated() {
        COUNTS.update(|value| value.map(|(a, b)| (a, b + 1)));
    }
    pub(crate) fn observe<T>(work: impl FnOnce() -> T) -> (T, (usize, usize)) {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                COUNTS.set(None);
            }
        }
        assert!(COUNTS.replace(Some((0, 0))).is_none());
        let _reset = Reset;
        let result = work();
        (result, COUNTS.get().unwrap())
    }
}
