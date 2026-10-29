//! Grain's effect implementation; the execution engine only sees declared work.

use super::*;
use crate::geometry::SourceRegion as Rect;

pub(crate) fn grain(
    input: Arc<ImageNode>,
    model: crate::grain::GrainParameters,
    seed: u64,
) -> Result<Arc<ImageNode>, Error> {
    if model.amplitude <= 0. {
        return Ok(input);
    }
    Ok(ImageNode::new(
        input.domain,
        Some(input),
        GrainOperation { model, seed },
    ))
}

fn raster(image: PlannedImage) -> crate::gpu::raster::Raster {
    image.domain.grid.raster(image.region)
}

struct GrainOperation {
    model: crate::grain::GrainParameters,
    seed: u64,
}

impl Operation for GrainOperation {
    fn input(&self, r: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(r))
    }

    fn in_place(&self) -> bool {
        true
    }

    fn lower(
        &self,
        f: &mut Fragment,
        _: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let key = crate::gpu::grain::FieldKey::new(raster(output), self.model, self.seed);
        let parameters = f.uniform(&crate::gpu::grain::parameter_data(key, self.model));
        let field = f.retained(RetainedKey::new(key), u64::from(output.cells()) * 16);
        let code: Arc<str> = crate::gpu::grain::shader().into();
        f.compute(
            code.clone(),
            "field",
            vec![(10, parameters.clone()), (12, Binding::Write(field))],
            output.cells(),
        );
        f.initialize(field);
        f.compute(
            code,
            "apply",
            vec![
                (1, Binding::Write(output.resource)),
                (10, parameters),
                (12, Binding::Write(field)),
            ],
            output.cells(),
        );
        Ok(())
    }
}
