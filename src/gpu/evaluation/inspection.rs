//! Source viewing is an ordinary captured colour operation, not a second photographic recipe.
use super::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Inspection {
    columns: [[f32; 4]; 3],
    coverage: [u32; 4],
}

impl std::hash::Hash for Inspection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for value in self.columns.into_iter().flatten() {
            crate::float_hash::f32(value, state);
        }
        self.coverage.hash(state);
    }
}

pub(crate) fn inspect(
    input: Value,
    parameters: crate::gpu::parameters::ProcessingParameters,
) -> Result<Value, Error> {
    let operation = Inspection {
        columns: parameters.post.map(|column| column.to_array()),
        coverage: [input.domain.grid.coverage, 0, 0, 0],
    };
    value::pointwise(
        input,
        value::Operation::pointwise_function(
            "source inspection v1",
            operation,
            Function::new(include_str!("inspection.wgsl"), "inspect_source")
                .with_uniform(0, &operation),
        ),
    )
}

#[cfg(test)]
pub(crate) fn has_inspection(image: &Value) -> bool {
    image
        .ancestors()
        .any(|node| node.parameters::<Inspection>().is_some())
}
