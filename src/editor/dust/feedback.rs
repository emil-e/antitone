//! Dust-only feedback queries the same healing endpoints used by image composition.
use super::*;

#[derive(Clone, PartialEq)]
struct Request {
    neutral: crate::image_recipe::NeutralRecipe,
    repairs: Vec<Repair>,
}

impl Request {
    fn new(recipe: &crate::image_recipe::ImageRecipe) -> Self {
        Self {
            neutral: recipe.neutral.clone(),
            repairs: recipe.repairs.clone(),
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    request: Option<Request>,
    results: Vec<RepairFeedback>,
}

impl State {
    pub(super) fn repair(
        &self,
        capture: &crate::image_recipe::ImageRecipe,
        repair: &Repair,
    ) -> RepairFeedback {
        self.request
            .as_ref()
            .filter(|request| request.neutral == capture.neutral)
            .and_then(|request| request.repairs.iter().position(|stored| stored == repair))
            .and_then(|index| self.results.get(index))
            .copied()
            .unwrap_or(RepairFeedback::Pending)
    }
}

impl DustTool {
    pub(super) fn update_feedback(&mut self, editor: &Editor, cx: &UiContext<'_, Editor>) {
        let Ok(recipe) = editor.positive_recipe(
            editor.input(),
            editor.document.project_snapshot().roll,
            editor.photo_settings(),
        ) else {
            return;
        };
        let wanted = Request::new(&recipe);
        if self.interaction.feedback.borrow().request.as_ref() == Some(&wanted) {
            return;
        }
        self.feedback_task = None;
        *self.interaction.feedback.borrow_mut() = State {
            request: Some(wanted.clone()),
            results: Vec::new(),
        };
        if wanted.repairs.is_empty() {
            return;
        }
        let Ok(source) = editor.capture_source_context() else {
            return;
        };
        let services = editor.source_services.clone();
        let state = self.interaction.feedback.clone();
        self.feedback_task = cx.spawn(move |cx| async move {
            let results = match source.await {
                Ok(context) => crate::background::run(move |cancelled| {
                    evaluate(context, recipe, services, &cancelled)
                })
                .await
                .ok()
                .and_then(Result::ok),
                Err(_) => None,
            };
            cx.update(|_| {
                let mut state = state.borrow_mut();
                if state.request.as_ref() == Some(&wanted) {
                    // A failed query is not evidence of a failed heal.
                    state.results = results.unwrap_or_default();
                }
            });
        });
    }
}

fn evaluate(
    context: source::Context,
    recipe: crate::image_recipe::ImageRecipe,
    services: Option<source::Services>,
    cancelled: &Cancellation,
) -> Result<Vec<RepairFeedback>, Error> {
    let stopped = || cancelled.is_cancelled();
    let scan = context.scan()?;
    let (scan, recipe) = crate::photo_preparation::native_image(&scan, &recipe)?;
    let services =
        services.ok_or_else(|| Error::Gpu("repair image-query services unavailable".into()))?;
    let source = crate::gpu::repair_source::Source::from_recipe(
        context.correction.gpu()?,
        services.evaluation(),
        &services.queue,
        &scan,
        &recipe,
    )?;
    collect_feedback(recipe.repairs.iter().map(|repair| {
        if stopped() {
            Err(Error::Cancelled)
        } else {
            source.repair_succeeded(repair, &stopped)
        }
    }))
}

fn collect_feedback(
    queries: impl IntoIterator<Item = Result<Option<bool>, Error>>,
) -> Result<Vec<RepairFeedback>, Error> {
    queries
        .into_iter()
        .map(|query| match query {
            Ok(Some(true)) => Ok(RepairFeedback::Ready),
            Ok(Some(false)) => Ok(RepairFeedback::Failed),
            Ok(None) | Err(Error::Cancelled) => Err(Error::Cancelled),
            Err(_) => Ok(RepairFeedback::Pending),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unavailable_query_keeps_other_explicit_repair_results() {
        let results = collect_feedback([
            Ok(Some(false)),
            Err(Error::Gpu("status query unavailable".into())),
            Ok(Some(true)),
        ])
        .unwrap();
        assert!(matches!(
            results.as_slice(),
            [
                RepairFeedback::Failed,
                RepairFeedback::Pending,
                RepairFeedback::Ready
            ]
        ));
        for cancelled in [Ok(None), Err(Error::Cancelled)] {
            let mut queried = 0;
            let results = collect_feedback(
                [Ok(Some(false)), cancelled, Ok(Some(true))]
                    .into_iter()
                    .inspect(|_| queried += 1),
            );
            assert!(matches!(results, Err(Error::Cancelled)));
            assert_eq!(queried, 2, "cancellation stops further queries");
        }
    }

    #[test]
    fn feedback_is_per_repair_and_retires_for_neutral_mask_or_donor_changes() {
        let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
        editor.set_dmin(Some(glam::DVec3::splat(0.8)));
        editor.set_dmax(Some(glam::DVec3::splat(0.02)));
        let source = editor.document.visit.current_source.unwrap();
        for x in [50, 80] {
            editor
                .dust_repairs_mut(source)
                .insert(
                    SupportMask::new([x, 50], [3, 3], vec![true; 9]).unwrap(),
                    RepairOrigin::Manual,
                    [30, 0],
                )
                .unwrap();
        }
        let recipe = editor
            .positive_recipe(
                editor.input(),
                editor.document.project_snapshot().roll,
                editor.photo_settings(),
            )
            .unwrap();
        let state = State {
            request: Some(Request::new(&recipe)),
            results: vec![RepairFeedback::Failed, RepairFeedback::Ready],
        };
        assert!(state.repair(&recipe, &recipe.repairs[0]) == RepairFeedback::Failed);
        assert!(state.repair(&recipe, &recipe.repairs[1]) == RepairFeedback::Ready);
        let mut graded = recipe.clone();
        graded.roll.exposure += 1.;
        assert!(state.repair(&graded, &graded.repairs[0]) == RepairFeedback::Failed);
        assert!(state.repair(&graded, &graded.repairs[1]) == RepairFeedback::Ready);

        let mut changed = recipe.clone();
        changed.repairs[0].offset[0] += 1;
        assert!(state.repair(&changed, &changed.repairs[0]) == RepairFeedback::Pending);
        let mut changed = recipe.clone();
        changed.repairs[0].support = SupportMask::new(
            [50, 50],
            [3, 3],
            vec![false, true, true, true, true, true, true, true, true],
        )
        .unwrap();
        assert!(state.repair(&changed, &changed.repairs[0]) == RepairFeedback::Pending);
        let mut changed = recipe.clone();
        let crate::processing::InputSettings::Negative { gamma, .. } = &mut changed.neutral.input
        else {
            panic!("fixture must supply a negative interpretation");
        };
        *gamma += 0.1;
        assert!(state.repair(&changed, &changed.repairs[0]) == RepairFeedback::Pending);
        assert!(state.repair(&changed, &changed.repairs[1]) == RepairFeedback::Pending);
    }
}
