//! Halation composes positive light, the shared Gaussian and signed recombination.
use super::*;
use crate::halation::HalationParameters;

pub(crate) fn halation(input: Value, parameters: HalationParameters) -> Result<Value, Error> {
    parameters.validate()?;
    if input.domain.meaning != Meaning::Graded {
        return Err(Error::InvalidParameter("halation requires graded AP1"));
    }
    if parameters.strength == 0. {
        return Ok(input);
    }
    let positive = value::pointwise(
        input.clone(),
        Meaning::Graded,
        value::Operation::new(
            "positive halation light v1",
            (),
            "fn pixel(cell:vec2i,position:vec2f,value:vec4f)->vec4f { return vec4f(max(value.rgb,vec3f(0.0)),value.w); }",
            "pixel",
            value::Access::Pointwise,
        ),
    )?;
    let blurred = value::gaussian(positive, parameters.sigma)?;
    Ok(Value::from_operation(
        input.domain,
        [input, blurred],
        Recombine(parameters),
    ))
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
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let alpha = self.0.fractions().map(|value| value as f32);
        let operation=value::Operation::new("signed halation recombination v1",self.0,
            "fn pixel(cell:vec2i,position:vec2f,source:vec4f,blurred:vec4f,alpha:vec4f)->vec4f { return vec4f(source.rgb+alpha.rgb*(blurred.rgb-max(source.rgb,vec3f(0.0))),source.w); }",
            "pixel",value::Access::Pointwise).with_uniform("vec4f",&[alpha[0],alpha[1],alpha[2],0.]);
        value::Calculation::new(
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
