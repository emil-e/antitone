//! Immutable mathematics, independent of a requested rectangle or its storage.
use super::*;

#[derive(Clone)]
pub struct Definition {
    pub(in crate::gpu::evaluation) content: Arc<Content>,
    pub(in crate::gpu::evaluation) domain: Domain,
    pub(in crate::gpu::evaluation) inputs: Vec<Value>,
    pub(in crate::gpu::evaluation) operation: Arc<dyn super::super::Operation>,
}

impl Drop for Definition {
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.inputs);
        while let Some(input) = pending.pop() {
            if let Some(mut definition) = Arc::into_inner(input.definition) {
                pending.append(&mut definition.inputs);
            }
        }
    }
}

impl From<&Value> for Value {
    fn from(value: &Value) -> Self {
        value.clone()
    }
}

impl std::ops::Deref for Value {
    type Target = Definition;
    fn deref(&self) -> &Self::Target {
        &self.definition
    }
}

impl Value {
    pub fn from_operation(
        domain: Domain,
        inputs: impl IntoIterator<Item = Value>,
        operation: impl super::super::Operation + 'static,
    ) -> Self {
        Self::captured(domain, inputs, Arc::new(operation))
    }

    pub(crate) fn captured(
        domain: Domain,
        inputs: impl IntoIterator<Item = Value>,
        operation: Arc<dyn super::super::Operation>,
    ) -> Self {
        #[cfg(test)]
        testing::created();
        let inputs: Vec<_> = inputs.into_iter().collect();
        // Operand views can carry regional aliases; capture their complete mathematics.
        let content = Content::new(
            domain,
            operation.key(),
            capture(operation.as_ref()),
            inputs.iter().map(|input| input.definition.content.clone()),
        );
        let access = operation.access();
        Self {
            definition: Arc::new(Definition {
                content,
                domain,
                inputs,
                operation,
            }),
            demand: None,
            access,
        }
    }

    #[cfg(test)]
    pub(crate) fn plan_unit(&self, output: CellRegion, budget: Budget) -> Result<WorkUnit, Error> {
        super::super::unit::plan(self, output, budget)
    }
    pub(crate) fn pointer(&self) -> *const Definition {
        Arc::as_ptr(&self.definition)
    }
    pub(crate) fn same_capture(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.definition, &other.definition)
    }
    /// Compare captured mathematics independently of requested regional content.
    pub fn same_definition(&self, other: &Self) -> bool {
        self.definition.content.same(&other.definition.content)
    }
    pub(crate) fn content(&self) -> Arc<Content> {
        self.demand
            .as_ref()
            .map_or(&self.definition.content, |output| &output.content)
            .clone()
    }
    pub(crate) fn operation(&self) -> &Arc<dyn super::super::Operation> {
        &self.operation
    }
    pub fn inputs(&self) -> &[Value] {
        &self.inputs
    }
    pub(crate) fn unary_input(&self) -> Option<&Value> {
        match self.inputs.as_slice() {
            [input] => Some(input),
            _ => None,
        }
    }
    pub fn source(&self) -> Option<Arc<dyn Source>> {
        self.operation.source()
    }
    pub(crate) fn parameters<T: std::any::Any>(&self) -> Option<&T> {
        let key = self.content.key.as_ref()?;
        key.get::<T>().or_else(|| super::parameters(key))
    }

    pub fn output(&self, region: CellRegion) -> Result<Output, Error> {
        self.describe_regions(region, &[region])
    }

    pub(crate) fn content_for(
        &self,
        region: CellRegion,
        requested: &[CellRegion],
    ) -> Result<Arc<Content>, Error> {
        Ok(self.describe_regions(region, requested)?.content())
    }

    fn describe_regions(
        &self,
        region: CellRegion,
        requested: &[CellRegion],
    ) -> Result<Output, Error> {
        if !self.domain.contains_region(region.validate()?) {
            return Err(Error::InvalidParameter(
                "value request exceeds its image domain",
            ));
        }
        for read in requested {
            if !region.contains_region(read.validate()?) {
                return Err(Error::InvalidParameter(
                    "described cells exceed the requested image region",
                ));
            }
        }
        let output = self.operation.describe(
            Output {
                domain: self.domain,
                region,
                content: self.content(),
            },
            &self.inputs,
            requested,
        )?;
        if output.domain != self.domain || output.region != region {
            return Err(Error::InvalidParameter(
                "image group changes its output domain or coverage",
            ));
        }
        Ok(output)
    }

    /// A demand selects coverage; it preserves the original mathematical definition.
    pub fn request(&self, region: CellRegion) -> Result<Value, Error> {
        let output = self.output(region)?;
        let mut value = self.clone().declare_output(output);
        value.demand.as_mut().unwrap().region = region;
        Ok(value)
    }

    pub(in crate::gpu::evaluation) fn lower(
        &self,
        region: CellRegion,
        budget: Budget,
    ) -> Result<Calculation, Error> {
        let requested = self.request(region)?;
        let calculation =
            match self
                .operation
                .clone()
                .calculation(&self.inputs, self.domain, region, budget)?
            {
                Some(calculation) => calculation,
                None => Calculation::from_value(requested.clone())?,
            };
        if calculation.domain() != self.domain {
            return Err(Error::InvalidParameter(
                "image group returned incompatible calculation coverage",
            ));
        }
        Ok(calculation.with_origin(requested))
    }

    pub(crate) fn ancestors(&self) -> impl Iterator<Item = &Value> {
        let mut pending = vec![self];
        let mut seen = std::collections::HashSet::new();
        std::iter::from_fn(move || {
            while let Some(value) = pending.pop() {
                if seen.insert(value.pointer()) {
                    pending.extend(value.inputs.iter().rev());
                    return Some(value);
                }
            }
            None
        })
    }

    /// Bounding rectangle for callers requiring one acquisition envelope.
    pub fn region_for(
        &self,
        ancestor: &Self,
        region: CellRegion,
    ) -> Result<Option<CellRegion>, Error> {
        let mut regions = self
            .regions_for(
                ancestor,
                region,
                Budget {
                    tile: [region.width, region.height],
                    scratch_bytes: u64::MAX,
                },
            )?
            .into_iter();
        let Some(mut cover) = regions.next() else {
            return Ok(None);
        };
        for next in regions {
            let low = [cover.x.min(next.x), cover.y.min(next.y)];
            let high = [
                cover.right().max(next.right()),
                cover.bottom().max(next.bottom()),
            ];
            let error = || Error::InvalidParameter("read coverage exceeds coordinate range");
            cover = CellRegion::new(
                low,
                [
                    (i64::from(high[0]) - i64::from(low[0]))
                        .try_into()
                        .map_err(|_| error())?,
                    (i64::from(high[1]) - i64::from(low[1]))
                        .try_into()
                        .map_err(|_| error())?,
                ],
            )
            .validate()?;
        }
        Ok(Some(cover))
    }

    pub fn regions_for(
        &self,
        ancestor: &Value,
        region: CellRegion,
        budget: Budget,
    ) -> Result<Vec<CellRegion>, Error> {
        if self.same_definition(ancestor) {
            return Ok(vec![region.validate()?]);
        }
        let mut required = Vec::new();
        for output in self.operation.work_units(region, budget)? {
            let calculation = super::super::unit::calculation(self, output, budget)?;
            if calculation
                .value()
                .is_some_and(|value| value.same_definition(ancestor))
            {
                required.push(output);
                continue;
            }
            let plan = calculation.compile(output, budget)?;
            let mut collect = |reads: Vec<Read>| -> Result<(), Error> {
                for read in reads {
                    let input = plan
                        .inputs
                        .get(read.input)
                        .ok_or(Error::InvalidParameter("read references an unknown input"))?;
                    required.extend(input.regions_for(ancestor, read.region, budget)?);
                }
                Ok(())
            };
            match plan.work {
                UnitWork::Source(_) => {}
                UnitWork::Cpu(work) => collect(work.reads)?,
                UnitWork::Gpu(work) => {
                    for phase in work.phases {
                        collect(phase?.reads)?;
                    }
                }
            }
        }
        Ok(required)
    }
}
