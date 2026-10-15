use super::*;
use crate::local_adjustments::{CONTRACT, LocalAdjustment, PhotoGradeSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AdjustmentId(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdjustmentRecord {
    pub id: AdjustmentId,
    pub name: String,
    pub adjustment: LocalAdjustment,
}

/// Source-owned processing order and non-reusing identity allocator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAdjustments {
    pub contract: String,
    pub next_id: u64,
    pub adjustments: Vec<AdjustmentRecord>,
}

impl Default for LocalAdjustments {
    fn default() -> Self {
        Self {
            contract: CONTRACT.into(),
            next_id: 1,
            adjustments: Vec::new(),
        }
    }
}

impl LocalAdjustments {
    pub fn validate(&self) -> Result<()> {
        require(
            self.contract == CONTRACT,
            "unsupported local-adjustment contract",
        )?;
        require(self.next_id > 0, "invalid local-adjustment ID allocator")?;
        let mut ids = HashSet::new();
        for record in &self.adjustments {
            require(
                record.id.0 > 0 && record.id.0 < self.next_id && ids.insert(record.id),
                "duplicate/invalid local-adjustment ID allocator",
            )?;
            require(
                !record.name.trim().is_empty(),
                "empty local-adjustment name",
            )?;
            record.adjustment.validate()?;
        }
        Ok(())
    }

    pub fn add(&mut self, name: String, adjustment: LocalAdjustment) -> Result<AdjustmentId> {
        self.validate()?;
        require(!name.trim().is_empty(), "empty local-adjustment name")?;
        adjustment.validate()?;
        let id = AdjustmentId(self.next_id);
        let next = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| failure("local-adjustment ID space exhausted"))?;
        self.adjustments.push(AdjustmentRecord {
            id,
            name,
            adjustment,
        });
        self.next_id = next;
        Ok(id)
    }

    pub fn remove(&mut self, id: AdjustmentId) -> Result<()> {
        let index = self
            .adjustments
            .iter()
            .position(|record| record.id == id)
            .ok_or_else(|| failure("unknown local-adjustment ID"))?;
        self.adjustments.remove(index);
        Ok(())
    }
}

impl Source {
    pub fn grade_snapshot(&self) -> Result<PhotoGradeSnapshot> {
        let locals = match &self.local_adjustments {
            Some(locals) => {
                locals.validate()?;
                locals
                    .adjustments
                    .iter()
                    .map(|record| record.adjustment)
                    .collect()
            }
            None => Vec::new(),
        };
        PhotoGradeSnapshot::new(self.photo, locals)
    }
}
