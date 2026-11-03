#[cfg(test)]
mod tests;

use crate::image_recipe::ImageRecipe;
use std::sync::Arc;

/// Active photographic intent and its original-reconstruction continuation.
/// Target, look and view changes supersede output demands within this owner.
#[derive(Default)]
pub(super) struct GradeScope {
    recipe: Option<Arc<ImageRecipe>>,
    pub original: super::OriginalView,
}

impl GradeScope {
    pub(super) fn recipe(&self) -> Option<Arc<ImageRecipe>> {
        self.recipe.clone()
    }

    fn matches(&self, recipe: Option<&ImageRecipe>) -> bool {
        match (self.recipe.as_deref(), recipe) {
            (None, None) => true,
            (Some(current), Some(recipe)) => current.same_photographic(recipe),
            _ => false,
        }
    }

    #[cfg(test)]
    fn new(recipe: Option<Arc<ImageRecipe>>) -> Self {
        Self {
            recipe,
            ..Self::default()
        }
    }
}

impl super::Editor {
    pub(super) fn observe_grade(&mut self, recipe: Option<Arc<ImageRecipe>>) {
        let grade = &mut self.active_canvas_mut().grade;
        if !grade.matches(recipe.as_deref()) {
            grade.original = super::OriginalView::default();
        }
        grade.recipe = recipe;
    }
}
