//! Prove all hoisted units against one traversal of the stage-declared read DAG.
use super::*;
use std::collections::{HashMap, HashSet};

#[cfg(all(test, feature = "editor"))]
mod tests;

struct Branch {
    targets: Vec<usize>,
    inputs: HashSet<usize>,
}

struct Required<'a> {
    targets: Vec<(&'a value::Value, Rect)>,
    exact: Option<HashMap<*const value::Definition, Vec<usize>>>,
    branches: HashMap<*const value::Definition, Branch>,
    visited: HashSet<(*const value::Definition, i32, i32, u32, u32)>,
    found: Vec<bool>,
    remaining: usize,
    budget: Budget,
    expanded: Vec<value::Value>,
}

impl Required<'_> {
    fn classify(&mut self, node: &value::Value) -> bool {
        let key = node.pointer();
        if let Some(branch) = self.branches.get(&key) {
            return !branch.targets.is_empty() || !branch.inputs.is_empty();
        }
        let targets = if let Some(exact) = &self.exact {
            exact.get(&key).cloned().unwrap_or_default()
        } else {
            self.targets
                .iter()
                .enumerate()
                .filter_map(|(index, (target, _))| node.same_definition(target).then_some(index))
                .collect()
        };
        let inputs = node
            .inputs
            .iter()
            .enumerate()
            .filter_map(|(index, input)| {
                let relevant = self.classify(input);
                (relevant || self.exact.is_none()).then_some(index)
            })
            .collect::<HashSet<_>>();
        let relevant = !targets.is_empty() || !inputs.is_empty();
        self.branches.insert(key, Branch { targets, inputs });
        relevant
    }

    fn reads(&mut self, inputs: &[value::Value], reads: Vec<Read>) -> Result<(), Error> {
        for read in reads {
            let input = inputs
                .get(read.input)
                .ok_or(Error::InvalidParameter("read references an unknown input"))?;
            self.expanded.push(input.clone());
            if self.classify(input) || self.exact.is_none() {
                self.visit(input, read.region)?;
            }
            if self.remaining == 0 {
                break;
            }
        }
        Ok(())
    }

    fn visit(&mut self, node: &value::Value, region: Rect) -> Result<(), Error> {
        region.validate()?;
        let key = node.pointer();
        if self.remaining == 0
            || !self
                .visited
                .insert((key, region.x, region.y, region.width, region.height))
        {
            return Ok(());
        }
        let branch = &self.branches[&key];
        let targets = branch.targets.clone();
        let descend = !branch.inputs.is_empty();
        for unit in node.operation.work_units(region, self.budget)? {
            let unit = unit.validate()?;
            for &index in &targets {
                if !self.found[index] && unit.contains_region(self.targets[index].1) {
                    self.found[index] = true;
                    self.remaining -= 1;
                }
            }
            if self.remaining == 0 {
                return Ok(());
            }
            if !descend {
                continue;
            }
            let plan = super::super::unit::plan(node, unit, self.budget)?;
            match plan.work {
                UnitWork::Source(_) => {}
                UnitWork::Cpu(work) => self.reads(&plan.inputs, work.reads)?,
                UnitWork::Gpu(work) => {
                    for phase in work.phases {
                        self.reads(&plan.inputs, phase?.reads)?;
                        if self.remaining == 0 {
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

pub(super) fn productions_are_required(
    root: &value::Value,
    region: Rect,
    work: &[Work],
    planning: Planning<'_>,
) -> Result<bool, Error> {
    let targets = work
        .iter()
        .filter_map(|work| match work {
            Work::Produce { image, output } => Some((image, output.region)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Ok(true);
    }
    let budget = Budget {
        tile: planning.tile,
        scratch_bytes: planning.scratch_bytes,
    };
    let mut exact = HashMap::<*const value::Definition, Vec<usize>>::new();
    for (index, (target, _)) in targets.iter().enumerate() {
        exact.entry(target.pointer()).or_default().push(index);
    }
    // Compiled dependencies normally point into this graph. Prove those together
    // without comparing every target definition at every ancestor.
    if matches!(
        prove(root, region, targets.clone(), budget, Some(exact)),
        Ok(true)
    ) {
        return Ok(true);
    }
    // Equivalent separately captured definitions still have the same semantics.
    prove(root, region, targets, budget, None)
}

fn prove(
    root: &value::Value,
    region: Rect,
    targets: Vec<(&value::Value, Rect)>,
    budget: Budget,
    exact: Option<HashMap<*const value::Definition, Vec<usize>>>,
) -> Result<bool, Error> {
    let mut required = Required {
        exact,
        found: vec![false; targets.len()],
        remaining: targets.len(),
        targets,
        branches: HashMap::new(),
        visited: HashSet::new(),
        budget,
        expanded: Vec::new(),
    };
    required.classify(root);
    required.visit(root, region)?;
    Ok(required.remaining == 0)
}
