use super::*;

struct DestinationStub;
impl Destination for DestinationStub {
    fn presentation(&self) -> Presentation {
        Presentation {
            action: "Return",
            opening: "Opening",
            working: "Returning",
            completed: "Returned",
        }
    }
    fn output(
        &self,
        _document: &Document,
        _target: OutputTarget,
    ) -> Result<output::Options, Error> {
        unreachable!("session ownership tests do not render")
    }
    fn save(&self, _document: &Document) -> Result<Document, Error> {
        unreachable!("session ownership tests do not save")
    }
    fn publish(&self, _document: &Document, _renders: &[Rendered]) -> Result<(), Error> {
        unreachable!("session ownership tests do not publish")
    }
}

fn document() -> Document {
    let mut editor = crate::editor::tests::fixture();
    crate::editor::tests::with_camera_basis(&mut editor);
    editor.document.capture()
}

#[test]
fn session_requires_distinct_ordered_sources() {
    let document = document();
    let id = document.project.order[0];
    let target = Arc::new(DestinationStub);
    assert!(Session::new(&document, vec![id], target.clone()).is_ok());
    for ids in [vec![], vec![id, id], vec![SourceId(u64::MAX)]] {
        assert!(Session::new(&document, ids, target.clone()).is_err());
    }
}

#[test]
fn project_snapshots_do_not_retain_the_callers_context_after_scope_retirement() {
    let document = document();
    let id = document.project.order[0];
    let target = Arc::new(DestinationStub);
    let weak = Arc::downgrade(&target);
    let mut scope = DocumentScope::from(document);
    let session = Session::new(&scope, vec![id], target).unwrap();
    scope.external_edit.session = Some(Arc::new(session));
    let snapshot = scope.capture();
    let replacement = scope.replacement(snapshot.clone());
    assert!(replacement.external_edit.session.is_none());
    assert!(
        weak.upgrade().is_some(),
        "the original document scope still owns the caller"
    );
    drop(scope);
    assert!(
        weak.upgrade().is_none(),
        "neither a snapshot nor its replacement owns the caller"
    );
    assert_eq!(snapshot.project.order, vec![id]);
}
