use super::*;
use crate::evaluation::Operation as Group;

pub fn source(source: Arc<dyn Source>) -> Value {
    super::super::source(source)
}

pub fn pointwise(input: Value, operation: Operation) -> Result<Value, Error> {
    if operation.access() != Access::Pointwise {
        return Err(Error::InvalidParameter(
            "pointwise node requires same-position input access",
        ));
    }
    Ok(Value::from_operation(
        input.domain(),
        [input],
        Pointwise(operation),
    ))
}
pub(super) struct Pointwise(pub(super) Operation);
impl Group for Pointwise {
    fn key(&self) -> Option<RetainedKey> {
        self.0.key.clone()
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, output)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::evaluation::Tiling,
    ) -> Result<Option<crate::evaluation::value::Calculation>, Error> {
        let inputs = crate::evaluation::value::Calculation::references(inputs);
        Ok(Some(Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            self.0.clone(),
        )?))
    }
}
