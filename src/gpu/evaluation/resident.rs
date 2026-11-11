//! Captured external arguments; the value compiler owns their physical access.

use super::*;

#[cfg(test)]
pub(crate) fn raster_input(domain: Domain) -> Value {
    Value::from_operation(domain, None, Resident(value::Operation::argument(true)))
}

pub(crate) fn borrowed_input(domain: Domain) -> Value {
    Value::from_operation(domain, None, Resident(value::Operation::argument(false)))
}

pub(super) struct Resident(pub(super) value::Operation);

impl Operation for Resident {
    fn reads(&self, _: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(Vec::new())
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        _: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        value::Calculation::new(domain, output, Vec::new(), self.0.clone()).map(Some)
    }
}
