//! Related image roots use the same preparation as GPU-recorded requests.
use super::*;

impl Context {
    pub(crate) fn materialize_many(
        &self,
        queue: &crate::gpu::submission::Queue,
        scope: &Materializations,
        evaluations: Vec<Evaluation>,
    ) -> Result<Vec<ImageFuture<ImageLayout>>, Error> {
        if !scope.belongs_to(self) {
            return Err(Error::InvalidParameter(
                "materializations belong to a different evaluation context",
            ));
        }
        let context = Self {
            products: scope.clone(),
            ..self.clone()
        };
        let prepared = context.prepare_images(evaluations)?;
        context.produce_images(queue, prepared)
    }
}
