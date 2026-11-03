//! Supported formation/printing combinations; execution is an ordinary LUT asset.
use super::{LookAsset, LookOrigin, monochrome, spektrafilm};
use crate::Error;
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Negative {
    Colour(spektrafilm::Negative),
    Monochrome(monochrome::Negative),
}
impl Negative {
    pub fn all() -> impl Iterator<Item = Self> {
        spektrafilm::Negative::all()
            .map(Self::Colour)
            .chain(monochrome::Negative::all().map(Self::Monochrome))
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Colour(v) => v.name(),
            Self::Monochrome(v) => v.name(),
        }
    }
    pub fn default_paper(self) -> Paper {
        match self {
            Self::Colour(_) => Paper::Colour(Default::default()),
            Self::Monochrome(_) => Paper::Monochrome(Default::default()),
        }
    }
}
impl From<spektrafilm::Negative> for Negative {
    fn from(value: spektrafilm::Negative) -> Self {
        Self::Colour(value)
    }
}
impl From<monochrome::Negative> for Negative {
    fn from(value: monochrome::Negative) -> Self {
        Self::Monochrome(value)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paper {
    Colour(spektrafilm::Paper),
    Monochrome(monochrome::Paper),
}
impl Paper {
    pub fn all() -> impl Iterator<Item = Self> {
        spektrafilm::Paper::all()
            .map(Self::Colour)
            .chain(monochrome::Paper::all().map(Self::Monochrome))
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Colour(v) => v.name(),
            Self::Monochrome(v) => v.name(),
        }
    }
    /// Current provider support, not a judgment of photographic plausibility.
    pub fn supports_negative(self, negative: Negative) -> bool {
        Selection::new(Some(negative), self).is_some()
    }
}
impl From<spektrafilm::Paper> for Paper {
    fn from(value: spektrafilm::Paper) -> Self {
        Self::Colour(value)
    }
}
impl From<monochrome::Paper> for Paper {
    fn from(value: monochrome::Paper) -> Self {
        Self::Monochrome(value)
    }
}

/// Each provider owns only its supported formation/printing relationships.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Colour(spektrafilm::Selection),
    Monochrome(monochrome::Selection),
}
impl From<spektrafilm::Selection> for Selection {
    fn from(value: spektrafilm::Selection) -> Self {
        Self::Colour(value)
    }
}
impl From<monochrome::Selection> for Selection {
    fn from(value: monochrome::Selection) -> Self {
        Self::Monochrome(value)
    }
}
impl Selection {
    pub fn new(negative: Option<Negative>, paper: Paper) -> Option<Self> {
        match (negative, paper) {
            (None, Paper::Colour(paper)) => Some(
                spektrafilm::Selection {
                    negative: None,
                    paper,
                }
                .into(),
            ),
            (Some(Negative::Colour(negative)), Paper::Colour(paper)) => Some(
                spektrafilm::Selection {
                    negative: Some(negative),
                    paper,
                }
                .into(),
            ),
            (None, Paper::Monochrome(paper)) => Some(monochrome::Selection::from(paper).into()),
            (Some(Negative::Monochrome(negative)), Paper::Monochrome(paper)) => Some(
                monochrome::Selection {
                    negative: Some(negative),
                    paper,
                }
                .into(),
            ),
            _ => None,
        }
    }
    pub fn from_origin(origin: &LookOrigin) -> Option<Self> {
        spektrafilm::Selection::from_origin(origin)
            .map(Self::Colour)
            .or_else(|| monochrome::Selection::from_origin(origin).map(Self::Monochrome))
    }
    pub fn origin(self) -> LookOrigin {
        match self {
            Self::Colour(v) => v.origin(),
            Self::Monochrome(v) => v.origin(),
        }
    }
    pub fn name(self) -> String {
        match self {
            Self::Colour(v) => v.name(),
            Self::Monochrome(v) => v.name(),
        }
    }
    pub fn negative(self) -> Option<Negative> {
        match self {
            Self::Colour(v) => v.negative.map(Negative::Colour),
            Self::Monochrome(v) => v.negative.map(Negative::Monochrome),
        }
    }
    pub fn paper(self) -> Paper {
        match self {
            Self::Colour(v) => Paper::Colour(v.paper),
            Self::Monochrome(v) => Paper::Monochrome(v.paper),
        }
    }
    pub fn bake(self, size: u32, cancel: &AtomicBool) -> Result<LookAsset, Error> {
        match self {
            Self::Colour(v) => spektrafilm::Model::new(v, cancel)?.bake(size, cancel),
            Self::Monochrome(v) => monochrome::Model::new(v).bake(size, cancel),
        }
    }
}
