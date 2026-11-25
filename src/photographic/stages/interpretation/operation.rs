//! Native camera response and working-space conversion.

use super::InputTransform;
use crate::{
    Error,
    gpu::{evaluation::*, parameters::ProcessingParameters},
    processing::{InputSettings, RollSettings},
};
use std::hash::{Hash, Hasher};

#[derive(Clone, Copy, Debug)]
pub enum Interpretation {
    Program(InputTransform),
    Packed(ProcessingParameters),
    Unavailable,
}

impl PartialEq for Interpretation {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Program(a), Self::Program(b)) => a == b,
            (Self::Packed(a), Self::Packed(b)) => bytemuck::bytes_of(a) == bytemuck::bytes_of(b),
            (Self::Unavailable, Self::Unavailable) => true,
            _ => false,
        }
    }
}

impl Hash for Interpretation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Program(program) => {
                std::mem::discriminant(&program.input()).hash(state);
                if let InputSettings::Negative {
                    dmin,
                    dmax,
                    response,
                    gamma,
                    balance,
                } = program.input()
                {
                    for anchor in [dmin, dmax] {
                        anchor.is_some().hash(state);
                        for channel in anchor.into_iter().flat_map(|value| value.to_array()) {
                            hash_f64(channel, state);
                        }
                    }
                    for control in [
                        response.toe,
                        response.shoulder,
                        gamma,
                        balance.warmth,
                        balance.tint,
                    ] {
                        hash_f64(control, state);
                    }
                }
                for coefficient in program.matrix().to_cols_array() {
                    hash_f64(coefficient, state);
                }
            }
            Self::Packed(parameters) => bytemuck::bytes_of(parameters).hash(state),
            Self::Unavailable => {}
        }
    }
}

impl Interpretation {
    fn parameters(&self) -> Result<ProcessingParameters, Error> {
        match self {
            Self::Program(program) => ProcessingParameters::interpret(
                program.input(),
                RollSettings::default(),
                glam::DMat3::IDENTITY,
            ),
            Self::Packed(parameters) => Ok(*parameters),
            Self::Unavailable => Err(Error::InvalidParameter(
                "neutral prefix requires captured input interpretation",
            )),
        }
    }
}

fn native_parameters(interpretation: &Interpretation) -> Result<ProcessingParameters, Error> {
    let parameters = interpretation.parameters()?;
    if parameters.graded() {
        Ok(parameters)
    } else {
        ProcessingParameters::digital(Default::default(), glam::DMat3::IDENTITY)
    }
}

pub struct Native {
    pub interpretation: Interpretation,
}

impl Native {
    pub fn new(interpretation: Interpretation) -> Result<Self, Error> {
        native_parameters(&interpretation)?;
        Ok(Self { interpretation })
    }
}

#[derive(Debug, PartialEq, Hash)]
pub(crate) struct NativeIdentity(pub Interpretation);

impl Operation for Native {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(NativeIdentity(self.interpretation)))
    }

    fn reads(&self, region: CellRegion) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
        Ok(vec![crate::gpu::evaluation::Read::new(0, region)])
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Tiling,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::pointwise_function(
                "native camera interpretation v1",
                NativeIdentity(self.interpretation),
                self.function(),
            ),
        )
        .map(Some)
    }
}

impl Native {
    pub(crate) fn function(&self) -> Function {
        let parameters =
            native_parameters(&self.interpretation).expect("captured interpretation was admitted");
        Function::new(
            format!(
                "{}\n{}",
                crate::photographic::stages::shader::functions(),
                include_str!("native.wgsl")
            ),
            "native_response",
        )
        .with_uniform(2, &parameters)
    }
}

pub struct Working {
    interpretation: Interpretation,
    matrix: glam::DMat3,
}

impl Working {
    pub fn new(interpretation: Interpretation, matrix: glam::DMat3) -> Result<Self, Error> {
        interpretation.parameters()?;
        crate::gpu::parameters::CameraMatrix::new(matrix)?;
        Ok(Self {
            interpretation,
            matrix,
        })
    }
}

#[derive(Debug, PartialEq)]
struct WorkingIdentity(Interpretation, glam::DMat3);

impl Hash for WorkingIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
        for coefficient in self.1.to_cols_array() {
            hash_f64(coefficient, state);
        }
    }
}

impl Operation for Working {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(WorkingIdentity(
            self.interpretation,
            self.matrix,
        )))
    }

    fn reads(&self, region: CellRegion) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
        Ok(vec![crate::gpu::evaluation::Read::new(0, region)])
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Tiling,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::pointwise_function(
                "native to working v1",
                WorkingIdentity(self.interpretation, self.matrix),
                self.function(),
            ),
        )
        .map(Some)
    }
}

impl Working {
    pub(crate) fn function(&self) -> Function {
        let parameters = self
            .interpretation
            .parameters()
            .expect("captured interpretation was admitted");
        let matrix = crate::gpu::parameters::CameraMatrix::new(self.matrix)
            .expect("captured camera matrix was admitted");
        Function::new(
            format!(
                "{}\n{}",
                crate::photographic::stages::shader::functions(),
                include_str!("working.wgsl")
            ),
            "native_to_working",
        )
        .with_uniform(2, &parameters)
        .with_uniform(3, &matrix)
    }
}
