use super::*;

pub(super) struct Nodes(Vec<Calculation>);
impl Nodes {
    pub(super) fn iter(&self) -> std::slice::Iter<'_, Calculation> {
        self.0.iter()
    }
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}
impl Drop for Nodes {
    fn drop(&mut self) {
        // Explicit order avoids recursive dependency retirement, including on errors.
        for node in self.0.drain(..) {
            drop(node);
        }
    }
}

pub(super) fn owned_nodes(root: Calculation, boundary: &HashMap<Id, usize>) -> (Shape, Id, Nodes) {
    let shape = root.shape();
    let mut seen = HashSet::new();
    let mut nodes = Vec::new();
    let mut stack = vec![(root.clone(), false)];
    while let Some((node, ready)) = stack.pop() {
        let key = node.pointer();
        if boundary.contains_key(&key) {
            continue;
        }
        if ready {
            nodes.push(node);
            continue;
        }
        if !seen.insert(key) {
            continue;
        }
        stack.push((node.clone(), true));
        for input in node.inputs().iter().rev() {
            stack.push((input.clone(), false));
        }
    }

    let pointer = root.pointer();
    drop(root);
    // A parent always retires while its children still have their own owners.
    nodes.reverse();
    (shape, pointer, Nodes(nodes))
}

pub(crate) fn retire(root: Calculation, boundaries: &[Calculation]) {
    let boundary = boundaries
        .iter()
        .enumerate()
        .map(|(i, node)| (node.pointer(), i))
        .collect::<HashMap<_, _>>();
    drop(owned_nodes(root, &boundary));
}
