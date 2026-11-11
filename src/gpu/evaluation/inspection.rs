//! Source viewing is an ordinary captured colour operation, not a second photographic recipe.
use super::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Inspection {
    columns: [[f32; 4]; 3],
    coverage: [u32; 4],
}

pub(crate) fn inspect(
    input: Value,
    parameters: crate::gpu::parameters::ProcessingParameters,
) -> Result<Value, Error> {
    if input.domain.meaning != Meaning::Inspection {
        return Err(Error::InvalidParameter(
            "inspection requires source working light",
        ));
    }
    let operation = Inspection {
        columns: parameters.post.map(|column| column.to_array()),
        coverage: [input.domain.grid.coverage, 0, 0, 0],
    };
    value::pointwise(
        input,
        Meaning::Inspection,
        value::Operation::pointwise_function(
            "source inspection v1",
            operation,
            Function::new(include_str!("inspection.wgsl"), "inspect_source")
                .with_uniform(0, &operation),
        ),
    )
}
