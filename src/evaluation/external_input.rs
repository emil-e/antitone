//! Image arguments whose pixels are bound by the caller rather than read by an operation.

use super::*;

#[cfg(test)]
pub(crate) fn raster_input(domain: Domain) -> Value {
    Value::from_operation(
        domain,
        None,
        ExternalInput(value::Operation::argument(true)),
    )
}

pub(crate) fn borrowed_input(domain: Domain) -> Value {
    Value::from_operation(
        domain,
        None,
        ExternalInput(value::Operation::argument(false)),
    )
}

pub(super) struct ExternalInput(pub(super) value::Operation);

impl Operation for ExternalInput {
    fn reads(&self, _: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(Vec::new())
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        _: &[crate::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::evaluation::Tiling,
    ) -> Result<Option<crate::evaluation::value::Calculation>, Error> {
        value::Calculation::new(domain, output, Vec::new(), self.0.clone()).map(Some)
    }
}
