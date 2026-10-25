use super::*;

#[test]
#[ignore = "Explicit complete private roll, native RAW development and Metal; read-only sources"]
fn complete_private_roll_prepares_reviews_analyzes_and_round_trips() {
    let evidence = PathBuf::from(
        std::env::var_os("ANTITONE_REGION_EVIDENCE").expect("ANTITONE_REGION_EVIDENCE"),
    );
    let raw_root = PathBuf::from(std::env::var_os("ANTITONE_RAW_ROOT").expect("ANTITONE_RAW_ROOT"));
    let name = std::env::var("ANTITONE_REGION_ROLL").expect("explicit ANTITONE_REGION_ROLL");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(evidence.join("roll-input/rolls.json")).unwrap())
            .unwrap();
    let roll = manifest["rolls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .expect("complete retained roll");
    let paths: Vec<_> = roll["photos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| raw_root.join(p.as_str().unwrap()))
        .collect();
    let revisions: Vec<_> = paths.iter().map(|p| thumbnails::FileKey::new(p)).collect();
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        editor.thumbnails = Thumbnails::default();
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(240),
            |editor, cx| editor.add_raws(paths.clone(), cx),
        );
        assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
        editor.sync_document();
        let before = editor.document.project.clone();
        let first = editor.current_source;
        let count = before.order.len();
        assert_eq!(count, paths.len());
        editor = crate::editor::tests::complete_analysis_preparation(
            editor,
            Duration::from_secs(240),
            |editor, cx| editor.request_analysis(cx),
        );
        assert!(
            editor.document.analysis_review.is_some(),
            "{}",
            editor.status
        );
        assert_eq!(editor.document.project, before);
        let approved = editor
            .document
            .analysis_review
            .as_ref()
            .unwrap()
            .approved
            .entries
            .clone();
        assert_eq!(approved.len(), count);
        assert!(
            approved.iter().all(|e| e.areas.ready()),
            "{name}: missing support"
        );
        let common: Vec<_> = approved
            .iter()
            .filter(|e| {
                e.areas.opening_shared && !e.areas.opening_exception && !e.areas.opening_edited
            })
            .collect();
        if let Some(first) = common.first() {
            assert!(
                common
                    .iter()
                    .all(|e| e.areas.opening == first.areas.opening),
                "{name}: shared opening differs between normal photos"
            );
        }
        editor = wait_with_limit(editor, Duration::from_secs(240), |e, cx| {
            e.browse_analysis_review(*before.order.last().unwrap(), cx)
        });
        assert_eq!(editor.document.project, before);
        editor = wait_with_limit(editor, Duration::from_secs(240), |e, cx| {
            e.confirm_analysis_review(cx);
            assert!(e.document.analysis_pending.is_some(), "{}", e.status);
        });
        assert!(editor.status.starts_with("Analyzed"), "{}", editor.status);
        assert_eq!(editor.current_source, first);
        let accepted = editor.document.project.clone();
        assert_eq!(accepted.order, before.order);
        assert_eq!(accepted.analysis_regions, approved);
        assert!(accepted.analysis_area.is_none());
        assert_eq!(accepted.orientation, before.orientation);
        assert_eq!(accepted.flat_field(), before.flat_field());
        eprintln!(
            "{name}: {} photos; Dmin {:?}; Dmax {:?}; Roll EV {}",
            count,
            accepted.dmin(),
            accepted.dmax(),
            accepted.roll.exposure
        );
        let temp =
            tempfile::tempdir_in(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".cache")).unwrap();
        let path = temp.path().join("review.antitone.json");
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(240),
            |editor, cx| editor.save_project(path.clone(), crate::project::Save::New, None, cx),
        );
        assert!(!editor.document.dirty(), "{}", editor.status);
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(240),
            |editor, cx| editor.start_document(Preparation::Open(path), cx),
        );
        assert_eq!(editor.document.project, accepted);
        editor = crate::editor::tests::complete_analysis_preparation(
            editor,
            Duration::from_secs(240),
            |editor, cx| editor.request_analysis(cx),
        );
        assert_eq!(
            editor
                .document
                .analysis_review
                .as_ref()
                .unwrap()
                .approved
                .entries,
            approved
        );
        editor.finish_pick(&crate::application::UiContext::render_only());
        assert_eq!(editor.document.project, accepted);
        assert!(!editor.document.dirty());
    });
    assert_eq!(
        paths
            .iter()
            .map(|p| thumbnails::FileKey::new(p))
            .collect::<Vec<_>>(),
        revisions
    );
}
