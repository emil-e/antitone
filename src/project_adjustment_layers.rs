use super::*;
use crate::adjustment_layers::{AdjustmentLayer, AdjustmentLayerSnapshot, CONTRACT};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AdjustmentLayerId(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdjustmentLayerRecord {
    pub id: AdjustmentLayerId,
    pub name: String,
    pub adjustment: AdjustmentLayer,
}

/// Source-owned processing order and non-reusing identity allocator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdjustmentLayers {
    pub contract: String,
    pub next_id: u64,
    pub layers: Vec<AdjustmentLayerRecord>,
}

impl Default for AdjustmentLayers {
    fn default() -> Self {
        Self {
            contract: CONTRACT.into(),
            next_id: 1,
            layers: Vec::new(),
        }
    }
}

impl AdjustmentLayers {
    pub fn with_global(photo: AdjustmentLayerSettings) -> Self {
        Self {
            contract: CONTRACT.into(),
            next_id: 2,
            layers: vec![AdjustmentLayerRecord {
                id: AdjustmentLayerId(1),
                name: "Global".into(),
                adjustment: AdjustmentLayer::global(photo),
            }],
        }
    }

    pub fn validate(&self) -> Result<()> {
        require(
            self.contract == CONTRACT,
            "unsupported adjustment-layer contract",
        )?;
        require(self.next_id > 0, "invalid adjustment-layer ID allocator")?;
        let mut ids = HashSet::new();
        for record in &self.layers {
            require(
                record.id.0 > 0 && record.id.0 < self.next_id && ids.insert(record.id),
                "duplicate/invalid adjustment-layer ID allocator",
            )?;
            require(
                !record.name.trim().is_empty(),
                "empty adjustment-layer name",
            )?;
            record.adjustment.validate()?;
        }
        Ok(())
    }

    pub fn add(&mut self, name: String, adjustment: AdjustmentLayer) -> Result<AdjustmentLayerId> {
        self.validate()?;
        require(!name.trim().is_empty(), "empty adjustment-layer name")?;
        adjustment.validate()?;
        let id = AdjustmentLayerId(self.next_id);
        let next = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| failure("adjustment-layer ID space exhausted"))?;
        self.layers.push(AdjustmentLayerRecord {
            id,
            name,
            adjustment,
        });
        self.next_id = next;
        Ok(id)
    }

    /// Stored order is bottom to top; creation inserts immediately above the selection.
    pub fn add_above(
        &mut self,
        above: Option<AdjustmentLayerId>,
        name: String,
        adjustment: AdjustmentLayer,
    ) -> Result<AdjustmentLayerId> {
        let index = above
            .map(|id| {
                self.layers
                    .iter()
                    .position(|record| record.id == id)
                    .map(|index| index + 1)
                    .ok_or_else(|| failure("unknown adjustment layer"))
            })
            .transpose()?
            .unwrap_or(self.layers.len());
        let id = self.add(name, adjustment)?;
        let record = self.layers.pop().expect("just added layer");
        self.layers.insert(index, record);
        Ok(id)
    }

    pub fn remove(&mut self, id: AdjustmentLayerId) -> Result<()> {
        let index = self
            .layers
            .iter()
            .position(|record| record.id == id)
            .ok_or_else(|| failure("unknown adjustment-layer ID"))?;
        self.layers.remove(index);
        Ok(())
    }
}

impl Source {
    pub fn grade_snapshot(&self) -> Result<AdjustmentLayerSnapshot> {
        let layers = match &self.adjustment_layers {
            Some(layers) => {
                layers.validate()?;
                layers
                    .layers
                    .iter()
                    .map(|record| record.adjustment)
                    .collect()
            }
            None => Vec::new(),
        };
        AdjustmentLayerSnapshot::new(layers)
    }
}
