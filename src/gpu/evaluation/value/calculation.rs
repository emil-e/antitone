//! Unit-local compiler expressions. Only logical inputs carry mathematical Value identity.
use super::*;

#[derive(Clone)]
pub struct Calculation(Operand);
#[derive(Clone)]
enum Operand {
    Input(Value),
    Node(Arc<Instruction>),
}
#[derive(Clone)]
struct Instruction {
    domain: Domain,
    region: CellRegion,
    inputs: Vec<Calculation>,
    primitive: Arc<compiler::Primitive>,
    access: Access,
    origin: Option<Value>,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::gpu::evaluation) enum Id {
    Value(*const Definition),
    Temporary(*const ()),
}
impl Drop for Instruction {
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.inputs);
        while let Some(input) = pending.pop() {
            if let Operand::Node(node) = input.0
                && let Some(mut instruction) = Arc::into_inner(node)
            {
                pending.append(&mut instruction.inputs);
            }
        }
    }
}
impl Calculation {
    pub(in crate::gpu::evaluation) fn compile(
        self,
        output: CellRegion,
        budget: Budget,
    ) -> Result<WorkUnit, Error> {
        super::compile_unit(&self, output, budget)
    }
    pub(crate) fn references(values: &[Value]) -> Vec<Self> {
        values.iter().cloned().map(Self::input).collect()
    }
    pub(crate) fn input(value: Value) -> Self {
        Self(Operand::Input(value))
    }
    pub(crate) fn new(
        domain: Domain,
        region: CellRegion,
        inputs: Vec<Self>,
        operation: Operation,
    ) -> Result<Self, Error> {
        #[cfg(test)]
        testing::calculated();
        let metadata = inputs.iter().map(Self::metadata).collect::<Vec<_>>();
        let (primitive, access) = compiler::Primitive::new(domain, region, &metadata, operation)?;
        Ok(Self(Operand::Node(Arc::new(Instruction {
            domain,
            region,
            inputs,
            primitive: Arc::new(primitive),
            access,
            origin: None,
        }))))
    }
    pub(crate) fn from_value(root: Value) -> Result<Self, Error> {
        use std::collections::{HashMap, HashSet};
        let mut pending = vec![(root.clone(), false)];
        let mut seen = HashSet::new();
        let mut lowered = HashMap::new();
        while let Some((value, ready)) = pending.pop() {
            let pointer = value.pointer();
            let applied = (value.operation().as_ref() as &dyn std::any::Any)
                .downcast_ref::<applied::Applied>();
            let internal = applied.is_some_and(|operation| {
                value.same_capture(&root)
                    || operation.inline()
                        && (value.shape() == Shape::Pair || !value.inputs().is_empty())
            });
            if !internal {
                lowered.entry(pointer).or_insert_with(|| Self::input(value));
                continue;
            }
            if ready {
                #[cfg(test)]
                testing::calculated();
                let applied = applied.unwrap();
                let inputs = value
                    .inputs()
                    .iter()
                    .map(|input| lowered[&input.pointer()].clone())
                    .collect::<Vec<_>>();
                let metadata = applied.primitive(value.domain(), value.coverage(), &inputs)?;
                lowered.insert(
                    pointer,
                    Self(Operand::Node(Arc::new(Instruction {
                        domain: value.domain(),
                        region: value.coverage(),
                        inputs,
                        primitive: Arc::new(metadata),
                        access: value.access,
                        origin: Some(value),
                    }))),
                );
                continue;
            }
            if !seen.insert(pointer) {
                continue;
            }
            if seen.len() > MAX_UNIT_VALUES {
                return Err(Error::WorkingAllowance(
                    "calculation exceeds the unit graph capacity",
                ));
            }
            pending.push((value.clone(), true));
            pending.extend(
                value
                    .inputs()
                    .iter()
                    .rev()
                    .cloned()
                    .map(|input| (input, false)),
            );
        }
        Ok(lowered.remove(&root.pointer()).unwrap())
    }
    pub(crate) fn with_origin(mut self, value: Value) -> Self {
        if let Operand::Node(node) = &mut self.0 {
            Arc::make_mut(node).origin = Some(value);
        }
        self
    }
    pub(crate) fn value(&self) -> Option<&Value> {
        match &self.0 {
            Operand::Input(value) => Some(value),
            _ => None,
        }
    }
    pub(in crate::gpu::evaluation) fn primitive(&self) -> Option<&Arc<compiler::Primitive>> {
        match &self.0 {
            Operand::Node(node) => Some(&node.primitive),
            _ => None,
        }
    }
    pub(in crate::gpu::evaluation) fn pointer(&self) -> Id {
        match &self.0 {
            Operand::Input(value) => Id::Value(value.pointer()),
            Operand::Node(node) => node.origin.as_ref().map_or_else(
                || Id::Temporary(Arc::as_ptr(node).cast()),
                |value| Id::Value(value.pointer()),
            ),
        }
    }
    pub(crate) fn inputs(&self) -> &[Self] {
        match &self.0 {
            Operand::Input(_) => &[],
            Operand::Node(node) => &node.inputs,
        }
    }
    pub(crate) fn input_value(&self, index: usize) -> Option<&Value> {
        if let Some(value) = self.value() {
            value.inputs().get(index)
        } else {
            self.inputs().get(index)?.value()
        }
    }
    pub(crate) fn domain(&self) -> Domain {
        match &self.0 {
            Operand::Input(value) => value.domain(),
            Operand::Node(node) => node.domain,
        }
    }
    pub(crate) fn coverage(&self) -> CellRegion {
        match &self.0 {
            Operand::Input(value) => value.coverage(),
            Operand::Node(node) => node.region,
        }
    }
    pub(crate) fn shape(&self) -> Shape {
        match &self.0 {
            Operand::Input(value) => value.shape(),
            Operand::Node(node) => node.primitive.shape(),
        }
    }
    pub(super) fn access(&self) -> Access {
        match &self.0 {
            Operand::Input(value) => value.access,
            Operand::Node(node) => node.access,
        }
    }
    fn metadata(&self) -> compiler::Input {
        compiler::Input {
            domain: self.domain(),
            coverage: self.coverage(),
            access: self.access(),
            shape: self.shape(),
        }
    }
    pub(crate) fn request(&self, region: CellRegion) -> Result<Self, Error> {
        match &self.0 {
            Operand::Input(value) => value.request(region).map(Self::input),
            Operand::Node(node) => {
                if !node.domain.contains_region(region.validate()?)
                    || !node.region.contains_region(region)
                {
                    return Err(Error::InvalidParameter(
                        "calculation demand exceeds its declared region",
                    ));
                }
                let mut result = self.clone();
                if let Operand::Node(node) = &mut result.0 {
                    Arc::make_mut(node).region = region;
                }
                Ok(result)
            }
        }
    }
    pub(crate) fn read(&self, region: CellRegion) -> Result<Self, Error> {
        self.request(region)
    }
    pub(crate) fn plan_unit(&self, output: CellRegion, budget: Budget) -> Result<WorkUnit, Error> {
        if self.value().is_some() {
            return Err(Error::InvalidParameter(
                "logical inputs require expression compilation",
            ));
        }
        let inputs = self
            .inputs()
            .iter()
            .map(|input| {
                input.value().cloned().ok_or(Error::InvalidParameter(
                    "constituents require graph compilation",
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.primitive()
            .unwrap()
            .clone()
            .plan_unit(&inputs, self.domain(), output, budget)
    }
    pub(crate) fn concatenate(mut inputs: Vec<Self>) -> Result<Self, Error> {
        if inputs.len() == 1
            && inputs[0].domain().meaning == Meaning::Arguments
            && inputs[0].coverage().x == 0
            && inputs[0].coverage().y == 0
            && inputs[0].coverage().height == 1
        {
            return Ok(inputs.pop().unwrap());
        }
        let mut cells = 0u32;
        for input in &inputs {
            let region = input.coverage();
            if input.shape() != Shape::Pixel {
                return Err(Error::InvalidParameter(
                    "concatenation requires pixel arguments",
                ));
            }
            cells = cells
                .checked_add(region.width.checked_mul(region.height).ok_or(
                    Error::InvalidParameter("concatenated arguments exceed GPU addressing"),
                )?)
                .ok_or(Error::InvalidParameter(
                    "concatenated arguments exceed GPU addressing",
                ))?;
        }
        let region = CellRegion::new([0, 0], [cells, 1]);
        Self::new(
            Domain {
                bounds: Some(region),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::Arguments,
            },
            region,
            inputs,
            Operation {
                key: None,
                body: Arc::new(Body::Concatenate),
            },
        )
    }
    pub(crate) fn window(domain: Domain, region: CellRegion, input: Self) -> Result<Self, Error> {
        Self::new(
            domain,
            region,
            vec![input],
            Operation {
                key: None,
                body: Arc::new(Body::Concatenate),
            },
        )
    }
    pub(crate) fn from_source(source: Arc<dyn Source>, region: CellRegion) -> Result<Self, Error> {
        Self::new(
            source.domain(),
            region,
            Vec::new(),
            Operation {
                key: None,
                body: Arc::new(Body::Source(source)),
            },
        )
    }
}
