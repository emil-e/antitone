//! Halation composes positive light, the shared Gaussian and signed recombination.

use super::{
    evaluation::{
        CellRegion, Domain, Operation, Read, RetainedKey, Tiling, Value,
        value::{self, Access, Calculation},
    },
    gaussian,
};
use crate::{Error, halation::HalationParameters};
use std::sync::Arc;

impl HalationParameters {
    pub(crate) fn compose(self, input: Value) -> Result<Value, Error> {
        self.validate()?;
        if self.strength == 0. {
            return Ok(input);
        }

        let positive = value::pointwise(
            input.clone(),
            value::Operation::new(
                "positive halation light v1",
                (),
                "fn pixel(cell:vec2i,position:vec2f,value:vec4f)->vec4f { return vec4f(max(value.rgb,vec3f(0.0)),value.w); }",
                "pixel",
                Access::Pointwise,
            ),
        )?;
        let blurred = gaussian::blur(positive, self.sigma)?;
        Ok(Value::from_operation(
            input.domain(),
            [input, blurred],
            Recombine(self),
        ))
    }
}

struct Recombine(HalationParameters);

impl Operation for Recombine {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.0))
    }

    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, output), Read::new(1, output)])
    }

    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: CellRegion,
        _budget: Tiling,
    ) -> Result<Option<Calculation>, Error> {
        let inputs = Calculation::references(inputs);
        let alpha = self.0.fractions().map(|value| value as f32);
        let operation = value::Operation::new(
            "signed halation recombination v1",
            self.0,
            "fn pixel(cell:vec2i,position:vec2f,source:vec4f,blurred:vec4f,alpha:vec4f)->vec4f { return vec4f(source.rgb+alpha.rgb*(blurred.rgb-max(source.rgb,vec3f(0.0))),source.w); }",
            "pixel",
            Access::Pointwise,
        )
        .with_uniform("vec4f", &[alpha[0], alpha[1], alpha[2], 0.]);

        Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?, inputs[1].request(output)?],
            operation,
        )
        .map(Some)
    }
}

#[cfg(test)]
mod tests;
