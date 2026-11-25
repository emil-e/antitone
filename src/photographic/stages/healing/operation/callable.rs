//! Gather demanded replacement arguments without an original-sized intermediate.
use super::*;

// Original-cell bins bound the work of each sample without a source-sized table.
// Entries store bin coordinates and a list of candidate replacement rectangles.
fn spatial_index(arguments: &[(CellRegion, u32)]) -> Result<Vec<[u32; 4]>, Error> {
    const BIN: i32 = 64;
    let mut bins = std::collections::HashMap::<[u32; 2], Vec<u32>>::new();
    for (index, (part, _)) in arguments.iter().enumerate() {
        let index = u32::try_from(index)
            .map_err(|_| Error::InvalidParameter("repair index exceeds GPU addressing"))?;
        for y in part.y.div_euclid(BIN)..=(part.bottom() - 1).div_euclid(BIN) {
            for x in part.x.div_euclid(BIN)..=(part.right() - 1).div_euclid(BIN) {
                bins.entry([x as u32, y as u32]).or_default().push(index);
            }
        }
    }
    let capacity = bins
        .len()
        .checked_mul(2)
        .and_then(usize::checked_next_power_of_two)
        .filter(|capacity| *capacity <= u32::MAX as usize)
        .ok_or(Error::InvalidParameter(
            "repair index exceeds GPU addressing",
        ))?;
    let mut records = vec![[0; 4]; capacity + 1];
    records[0] = [capacity as u32, records.len() as u32, BIN as u32, 0];
    records.extend(
        arguments
            .iter()
            .flat_map(|(part, offset)| [part.packed(), [*offset, 0, 0, 0]]),
    );
    for ([x, y], candidates) in bins {
        let hash = x.wrapping_mul(0x9e3779b1) ^ y.wrapping_mul(0x85ebca77);
        let mut slot = hash as usize & (capacity - 1);
        while records[1 + slot][3] != 0 {
            slot = (slot + 1) & (capacity - 1);
        }
        let offset = u32::try_from(records.len())
            .map_err(|_| Error::InvalidParameter("repair index exceeds GPU addressing"))?;
        let count = u32::try_from(candidates.len())
            .map_err(|_| Error::InvalidParameter("repair index exceeds GPU addressing"))?;
        records[1 + slot] = [x, y, offset, count];
        records.extend(candidates.into_iter().map(|index| [index, 0, 0, 0]));
    }
    u32::try_from(records.len())
        .map_err(|_| Error::InvalidParameter("repair index exceeds GPU addressing"))?;
    Ok(records)
}

impl Compose {
    pub(super) fn express(
        &self,
        inputs: &[value::Calculation],
        domain: Domain,
        output: CellRegion,
    ) -> Result<value::Calculation, Error> {
        let base = inputs[0].request(output)?;
        let selected = self
            .influencing(output)
            .map(|(index, support, part)| (index, support.clone(), part))
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Ok(base);
        }
        let mut replacements = Vec::new();
        let mut arguments = Vec::new();
        let mut cells = 0u32;
        let mut supports = Vec::new();
        for (index, support, part) in selected {
            arguments.push((part, cells));
            cells =
                cells
                    .checked_add(part.width.checked_mul(part.height).ok_or(
                        Error::InvalidParameter("repair arguments exceed GPU addressing"),
                    )?)
                    .ok_or(Error::InvalidParameter(
                        "repair arguments exceed GPU addressing",
                    ))?;
            replacements.push(inputs[index].request(part)?.read(part)?);
            supports.push(support);
        }
        let packed = value::Calculation::concatenate(replacements)?;
        let mut records = spatial_index(&arguments)?;
        let mut masks = vec![[0u32; 4]; (cells as usize).div_ceil(128)];
        for ((part, offset), support) in arguments.iter().zip(&supports) {
            for y in part.y..part.bottom() {
                for x in part.x..part.right() {
                    if support.contains([x as u32, y as u32]) {
                        let i = *offset as usize + part.index([x, y]);
                        masks[i / 128][i / 32 % 4] |= 1 << (i % 32);
                    }
                }
            }
        }
        records[0][3] = u32::try_from(records.len())
            .map_err(|_| Error::InvalidParameter("repair index exceeds GPU addressing"))?;
        records.extend(masks);
        value::Calculation::new(
            domain,
            output,
            vec![base, packed],
            value::Operation::new(
                "selected replacement composition v1",
                supports,
                include_str!("../compose.wgsl"),
                "pixel",
                value::Access::Neighborhood,
            )
            .with_table(records)
            .with_input_access(vec![value::Access::Pointwise, value::Access::Neighborhood]),
        )
    }
}
