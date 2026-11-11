//! Captured kernel mathematics and dependency rules, without a requested execution layout.
use super::*;

#[derive(Clone)]
pub(super) struct Applied {
    pub operation: Operation,
    reads: Vec<Option<CellRegion>>,
    natural: Option<CellRegion>,
    access: Access,
}
impl Applied {
    pub(super) fn capture(
        domain: Domain,
        region: CellRegion,
        inputs: &[Value],
        operation: Operation,
    ) -> Result<Self, Error> {
        let metadata = inputs
            .iter()
            .map(|input| compiler::Input {
                domain: input.domain(),
                coverage: input.coverage(),
                access: input.access,
                shape: input.shape(),
            })
            .collect::<Vec<_>>();
        let (primitive, access) = compiler::Primitive::new(domain, region, &metadata, operation)?;
        let copy = matches!(primitive.operation.body.as_ref(), Body::Concatenate)
            && inputs.len() == 1
            && inputs[0].coverage() == region
            && inputs[0].domain().grid == domain.grid;
        let reads = primitive
            .reads
            .iter()
            .enumerate()
            .map(|(index, region)| {
                if copy || primitive.operation.input_access(index) == Access::Pointwise {
                    None
                } else {
                    Some(*region)
                }
            })
            .collect();
        let natural = (matches!(primitive.operation.body.as_ref(), Body::Cpu(_))
            || matches!(primitive.operation.body.as_ref(), Body::Concatenate) && !copy)
            .then_some(region);
        Ok(Self {
            operation: primitive.operation,
            reads,
            natural,
            access,
        })
    }
    pub(super) fn shape(&self) -> Shape {
        match self.operation.body.as_ref() {
            Body::Gpu(body) => body.shape,
            _ => Shape::Pixel,
        }
    }
    pub(super) fn read_regions(&self, output: CellRegion) -> Vec<CellRegion> {
        self.reads
            .iter()
            .map(|region| region.unwrap_or(output))
            .collect()
    }
    pub(super) fn inline(&self) -> bool {
        matches!(
            self.operation.body.as_ref(),
            Body::Gpu(_) | Body::Concatenate | Body::Failed(_)
        )
    }
    pub(super) fn primitive(
        &self,
        domain: Domain,
        region: CellRegion,
        inputs: &[Calculation],
    ) -> Result<compiler::Primitive, Error> {
        let output = self.natural.unwrap_or(region);
        let reads = self.read_regions(output);
        let metadata = inputs
            .iter()
            .zip(&reads)
            .map(|(input, read)| compiler::Input {
                domain: input.domain(),
                coverage: *read,
                access: input.access(),
                shape: input.shape(),
            })
            .collect::<Vec<_>>();
        let (mut primitive, _) =
            compiler::Primitive::new(domain, output, &metadata, self.operation.clone())?;
        primitive.extent = self.natural.or(domain.bounds);
        Ok(primitive)
    }
}
impl super::super::Operation for Applied {
    fn key(&self) -> Option<RetainedKey> {
        self.operation.key.clone()
    }
    fn access(&self) -> Access {
        self.access
    }
    fn source(&self) -> Option<Arc<dyn Source>> {
        match self.operation.body.as_ref() {
            Body::Source(source) => Some(source.clone()),
            _ => None,
        }
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(self
            .read_regions(output)
            .into_iter()
            .enumerate()
            .map(|(input, region)| Read::new(input, region))
            .collect())
    }
    fn work_units(&self, output: CellRegion, _: Budget) -> Result<WorkUnits, Error> {
        if let Some(region) = self.natural {
            if !region.contains_region(output) {
                return Err(Error::InvalidParameter(
                    "pixel request exceeds its declared coverage",
                ));
            }
            return Ok(Box::new(std::iter::once(region)));
        }
        Ok(Box::new(std::iter::once(output)))
    }
}
