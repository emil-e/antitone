use super::*;
use crate::application::test_support::with_application;
use crate::editor::dust::worker::testing::Gate;
use std::time::Duration;

#[test]
fn stroke_tasks_return_their_outcome_and_completion_withdraws_pending_geometry() {
    for choice in [
        Ok(SearchOutcome::Candidate([20, 0])),
        Ok(SearchOutcome::Impossible),
        Err("authored search failure".to_owned()),
    ] {
        let mut editor = crate::editor::tests::fixture();
        crate::editor::tests::add_ready_fixture(&mut editor);
        editor.document.source = false;
        editor.tab = 1;
        let gate = Gate::install(&mut editor);
        let expected = match &choice {
            Ok(SearchOutcome::Candidate(_)) => Ok(Some(RepairId(1))),
            Ok(_) => Ok(None),
            Err(error) => Err(error.clone()),
        };
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.begin_dust_mask(None);
                editor
                    .document
                    .visit
                    .dust
                    .mask_draft
                    .as_mut()
                    .unwrap()
                    .pixels = (80..83)
                    .flat_map(|y| (80..83).map(move |x| [x, y]))
                    .collect();
                editor.commit_dust_mask(cx);
                assert!(editor.dust_search_pending());
            });
            gate.next(app).complete(choice);
            app.run_until(Duration::from_secs(5), |editor| {
                !editor.dust_search_pending()
            });
            app.update(|editor| {
                assert!(editor.pending_dust_strokes().is_empty());
                let task = &mut editor.document.visit.dust.searches.tasks[0].task;
                assert!(task.is_finished());
                let actual = futures_lite::future::block_on(futures_lite::future::poll_once(task));
                assert_eq!(actual, Some(expected.clone()));
                let source = editor.document.visit.current_source.unwrap();
                assert_eq!(
                    editor.dust_repairs(source).is_some(),
                    matches!(expected, Ok(Some(_)))
                );
                editor.document.visit.dust.searches.prune();
                assert!(editor.document.visit.dust.searches.tasks.is_empty());
            });
        });
    }
}
