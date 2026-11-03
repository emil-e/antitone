use super::*;
use std::{cell::Cell, rc::Rc};

mod navigation;

struct Probe {
    dropped: Rc<Cell<usize>>,
    actions: Rc<Cell<usize>>,
    first_config: Rc<Cell<Option<usize>>>,
    reset: Option<bool>,
    activity: Option<TaskRequest>,
}
impl Probe {
    fn new(dropped: &Rc<Cell<usize>>, actions: &Rc<Cell<usize>>) -> Self {
        Self {
            dropped: dropped.clone(),
            actions: actions.clone(),
            first_config: Rc::new(Cell::new(None)),
            reset: None,
            activity: None,
        }
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.dropped.set(self.dropped.get() + 1);
    }
}
impl Tool for Probe {
    fn presentation(&self, _editor: &Editor) -> Presentation {
        Presentation {
            identity: presentation::ToolIdentity {
                icon: icons::Icon::Crop,
                title: "Probe",
            },
            primary: None,
            dismiss: "Done",
            fit_label: "Fit",
            warning: None,
            help: "",
            navigation_help: None,
        }
    }
    fn composition(&self, normal: Composition) -> Composition {
        if self.first_config.get().is_none() {
            self.first_config.set(Some(self.dropped.get()));
        }
        normal
    }
    fn drawer(&mut self, ui: &mut egui::Ui, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        presentation::ToolIdentity {
            icon: icons::Icon::Pick,
            title: "Reference bounds",
        }
        .inspector(
            ui,
            |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let reset = self.reset.is_some_and(|enabled| {
                        icons::button(
                            ui,
                            icons::Icon::Reset,
                            enabled,
                            "Reset reference bounds",
                            "Reset reference bounds.",
                        )
                        .clicked()
                    });
                    ui.weak("4 regions");
                    reset
                })
                .inner
            },
            |ui, reset| {
                ui.label("Probe body");
                if reset {
                    self.actions.set(self.actions.get() + 1);
                    editor.status = "Probe draft reset".into();
                }
            },
        );
    }
    fn activity(&self, _editor: &Editor) -> Option<TaskRequest> {
        self.activity
    }
    fn action(&mut self, _action: Action, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        self.actions.set(self.actions.get() + 1);
        editor.end_tool();
    }
}

#[test]
fn tool_composes_its_drawer_and_handles_enabled_header_actions() {
    for enabled in [true, false] {
        for reset in [None, Some(false), Some(true)] {
            let actions = Rc::new(Cell::new(0));
            let dropped = Rc::new(Cell::new(0));
            let mut probe = Probe::new(&dropped, &actions);
            probe.reset = reset;
            let mut editor = Editor::state();
            assert!(editor.activate_tool(Placement::Photo, Box::new(probe)));
            crate::application::test_support::with_application(editor, |app| {
                let context = egui::Context::default();
                context.enable_accesskit();
                let render = |events| {
                    app.enter(|editor, cx| {
                        let mut output = context.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(300., 240.),
                                )),
                                events,
                                ..Default::default()
                            },
                            |ui| {
                                if !enabled {
                                    ui.disable();
                                }
                                editor.hosted_drawer(ui, cx);
                            },
                        );
                        output.textures_delta.clear();
                        output
                    })
                };
                render(vec![]);
                let output = render(vec![]);
                let nodes = &output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes;
                assert!(
                    nodes
                        .iter()
                        .any(|(_, node)| node.role() == egui::accesskit::Role::Label
                            && node.label().or_else(|| node.value()) == Some("Reference bounds"))
                );
                assert!(
                    nodes
                        .iter()
                        .any(|(_, node)| node.role() == egui::accesskit::Role::Label
                            && node.label().or_else(|| node.value()) == Some("Probe body"))
                );
                assert!(nodes.iter().any(|(_, node)| {
                    node.role() == egui::accesskit::Role::Label
                        && node.label().or_else(|| node.value()) == Some("4 regions")
                }));
                let button = nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some("Reset reference bounds"));
                assert_eq!(button.is_some(), reset.is_some());
                if let Some((_, button)) = button {
                    assert_eq!(button.is_disabled(), !enabled || reset != Some(true));
                    let bounds = button.bounds().unwrap();
                    let position = egui::pos2(
                        ((bounds.x0 + bounds.x1) / 2.) as f32,
                        ((bounds.y0 + bounds.y1) / 2.) as f32,
                    );
                    for pressed in [true, false] {
                        render(vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]);
                    }
                }
                assert_eq!(actions.get(), usize::from(enabled && reset == Some(true)));
                app.read(|editor| {
                    assert!(
                        editor.has_hosted_tool(),
                        "reset intent does not end the interaction"
                    );
                    if enabled && reset == Some(true) {
                        assert_eq!(editor.status, "Probe draft reset");
                    }
                });
            });
        }
    }
}

struct Candidate(Crop);
impl Tool for Candidate {
    fn presentation(&self, _editor: &Editor) -> Presentation {
        Presentation {
            identity: presentation::ToolIdentity {
                icon: icons::Icon::Crop,
                title: "Candidate",
            },
            primary: Some("Accept".into()),
            dismiss: "Discard",
            fit_label: "Fit",
            warning: None,
            help: "",
            navigation_help: None,
        }
    }
    fn action(&mut self, action: Action, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        if matches!(action, Action::Dismiss) || editor.accept_crop(self.0).is_ok() {
            editor.end_tool();
        }
    }
}

#[test]
fn photo_geometry_edit_rejects_before_mutation_and_records_valid_acceptance() {
    let mut editor = Editor::state();
    super::super::tests::add_ready_fixture(&mut editor);
    editor.sync_document();
    let before = editor.document.state.project.clone();
    let viewport = editor.document.visit.canvas.viewport;
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let invalid = Crop {
        center: [-500., -500.],
        size: [400, 300],
        angle: 0.,
    };
    assert!(editor.activate_tool(Placement::Photo, Box::new(Candidate(invalid))));
    editor.hosted_action(Action::Primary);
    assert!(
        editor.current_tool().is_some(),
        "rejection leaves the interaction available"
    );
    assert!(editor.document.visit.crop.is_none());
    assert_eq!(editor.document.visit.canvas.viewport, viewport);
    assert_eq!(editor.document.state.project, before);
    assert!(editor.history_label(history::Direction::Undo).is_none());
    let valid = Crop {
        size: [400, 300],
        ..Crop::from_region(area)
    };
    assert!(editor.activate_tool(Placement::Photo, Box::new(Candidate(valid))));
    editor.hosted_action(Action::Primary);
    assert!(editor.current_tool().is_none());
    assert_eq!(editor.document.visit.crop, Some(valid));
    assert_eq!(
        editor
            .document
            .state
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap()
            .crop,
        Some(valid)
    );
    assert_eq!(editor.history_label(history::Direction::Undo), Some("Crop"));
}

#[test]
fn activation_does_not_query_composition_or_require_a_projection_to_initialize_the_tool() {
    for placement in [Placement::Document, Placement::Photo] {
        let mut editor = Editor::state();
        crate::editor::tests::add_ready_fixture(&mut editor);
        let dropped = Rc::new(Cell::new(0));
        let actions = Rc::new(Cell::new(0));
        let probe = Probe::new(&dropped, &actions);
        let projected = probe.first_config.clone();
        assert!(editor.activate_tool(placement, Box::new(probe)));
        assert_eq!(
            projected.get(),
            None,
            "activation must not calculate a view"
        );
        let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
        let normal = editor.normal_canvas_composition(area, None);
        assert_eq!(editor.canvas_composition(area, None), normal);
        assert_eq!(
            projected.get(),
            Some(0),
            "the explicit query must reach the tool"
        );
    }
}

#[test]
fn document_and_photo_slots_follow_their_scopes_without_synthetic_actions() {
    let mut editor = Editor::state();
    let dropped = Rc::new(Cell::new(0));
    let actions = Rc::new(Cell::new(0));
    assert!(editor.activate_tool(
        Placement::Document,
        Box::new(Probe::new(&dropped, &actions))
    ));
    editor.replace_photo_scope(1);
    assert!(editor.current_tool().is_some());
    assert_eq!(dropped.get(), 0);

    let photo = Probe::new(&dropped, &actions);
    let configured = photo.first_config.clone();
    assert_eq!(configured.get(), None);
    assert!(editor.activate_tool(Placement::Photo, Box::new(photo)));
    assert_eq!(dropped.get(), 1);
    editor.canvas_composition(
        Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        },
        None,
    );
    assert_eq!(configured.get(), Some(1));
    editor.replace_photo_scope(2);
    assert_eq!(dropped.get(), 2);
    assert!(
        editor.current_tool().is_none(),
        "no retained document predecessor resumes"
    );
    assert_eq!(actions.get(), 0);

    assert!(editor.activate_tool(
        Placement::Document,
        Box::new(Probe::new(&dropped, &actions))
    ));
    let captured = editor.document.capture();
    editor.document = DocumentScope::from(document::Document::default());
    assert_eq!(dropped.get(), 3);
    assert!(editor.current_tool().is_none());
    assert_eq!(actions.get(), 0);
    drop(captured);
}

#[test]
fn photo_departure_retires_only_its_hosted_activity_while_document_activity_survives() {
    for placement in [Placement::Document, Placement::Photo] {
        for executing in [false, true] {
            for matching_activity in [false, true] {
                let mut editor = Editor::state();
                let dropped = Rc::new(Cell::new(0));
                let actions = Rc::new(Cell::new(0));
                let task = Task::Regions(73);
                let mut probe = Probe::new(&dropped, &actions);
                probe.activity = Some(task.request());
                assert!(editor.activate_tool(placement, Box::new(probe)));
                let foreground = if matching_activity {
                    task
                } else {
                    Task::Save(73)
                };
                editor.tasks.begin(foreground);
                editor.busy = true;
                assert!(editor.tasks.is_current(foreground.request()));
                let document_owned = matches!(placement, Placement::Document);
                let activity_survives = document_owned || !matching_activity;
                let depart = |editor: &mut Editor| {
                    editor.replace_photo_scope(1);
                    assert_eq!(
                        editor.tasks.is_current(foreground.request()),
                        activity_survives
                    );
                    assert_eq!(editor.busy, activity_survives);
                };
                if executing {
                    editor
                        .with_current_tool(|_, editor| depart(editor))
                        .unwrap();
                } else {
                    depart(&mut editor);
                }
                assert_eq!(editor.has_hosted_tool(), document_owned);
                assert_eq!(dropped.get(), usize::from(!document_owned));
                assert_eq!(
                    actions.get(),
                    0,
                    "scope departure must not synthesize an action"
                );
                editor.end_tool();
                assert_eq!(dropped.get(), 1);
                assert_eq!(
                    editor.tasks.is_current(foreground.request()),
                    !matching_activity
                );
                assert_eq!(editor.busy, !matching_activity);
            }
        }
    }
}

#[test]
fn legacy_and_hosted_installation_are_exclusive_in_both_directions() {
    let mut editor = Editor::state();
    let dropped = Rc::new(Cell::new(0));
    let actions = Rc::new(Cell::new(0));
    assert!(editor.activate_tool(Placement::Photo, Box::new(Probe::new(&dropped, &actions))));
    editor
        .document
        .set_analysis_operation(Some(analysis_workflow::Operation::new(
            analysis_workflow::Phase::Review,
        )));
    assert_eq!(dropped.get(), 1);
    assert!(editor.current_tool().is_none());
    assert!(matches!(
        editor.document.tool.as_ref(),
        Some(legacy_tool::Tool::Analyze(_))
    ));
    assert!(editor.activate_tool(
        Placement::Document,
        Box::new(Probe::new(&dropped, &actions))
    ));
    assert!(editor.document.tool.is_none());
    assert!(editor.current_tool().is_some());
    assert_eq!(actions.get(), 0);
    editor.hosted_action(Action::Dismiss);
    assert_eq!(actions.get(), 1);
    assert_eq!(dropped.get(), 2);
    assert!(editor.current_tool().is_none());
}

#[test]
fn rejected_admission_drops_candidate_without_configuring_or_replacing_the_owner() {
    let mut editor = Editor::state();
    let dropped = Rc::new(Cell::new(0));
    let actions = Rc::new(Cell::new(0));
    assert!(editor.activate_tool(
        Placement::Document,
        Box::new(Probe::new(&dropped, &actions))
    ));
    let candidate = Probe::new(&dropped, &actions);
    let configured = candidate.first_config.clone();
    editor.busy = true;
    assert!(!editor.activate_tool(Placement::Photo, Box::new(candidate)));
    assert_eq!(dropped.get(), 1);
    assert_eq!(configured.get(), None);
    assert!(editor.current_tool().is_some());
    assert_eq!(actions.get(), 0);
    editor.busy = false;
    editor.hosted_action(Action::Dismiss);
    assert_eq!(dropped.get(), 2);
    assert_eq!(actions.get(), 1);
}

#[test]
fn executing_tool_is_active_and_can_replace_or_end_current_without_resurrection() {
    for origin in [Placement::Document, Placement::Photo] {
        for destination in [Placement::Document, Placement::Photo] {
            let mut editor = Editor::state();
            let dropped = Rc::new(Cell::new(0));
            let actions = Rc::new(Cell::new(0));
            assert!(editor.activate_tool(origin, Box::new(Probe::new(&dropped, &actions))));
            editor
                .with_current_tool(|_, editor| {
                    assert!(editor.has_hosted_tool());
                    assert!(editor.current_tool().unwrap().is::<Probe>());
                    assert!(editor.with_current_tool(|_, _| ()).is_none());
                    assert!(
                        editor.activate_tool(destination, Box::new(Probe::new(&dropped, &actions)))
                    );
                    assert_eq!(
                        dropped.get(),
                        0,
                        "executing predecessor is still on the stack"
                    );
                    editor.end_tool();
                    assert_eq!(dropped.get(), 1, "end targets the installed replacement");
                    assert!(editor.current_tool().is_none());
                })
                .unwrap();
            assert_eq!(dropped.get(), 2);
            assert_eq!(
                actions.get(),
                0,
                "replacement/end do not synthesize actions"
            );
            assert!(editor.current_tool().is_none());
        }
    }
}

#[test]
fn scope_replacement_during_callback_cannot_restore_into_the_new_owner() {
    for placement in [Placement::Document, Placement::Photo] {
        let mut editor = Editor::state();
        let dropped = Rc::new(Cell::new(0));
        let actions = Rc::new(Cell::new(0));
        assert!(editor.activate_tool(placement, Box::new(Probe::new(&dropped, &actions))));
        editor
            .with_current_tool(|_, editor| {
                if matches!(placement, Placement::Photo) {
                    editor.replace_photo_scope(42);
                } else {
                    editor.document = DocumentScope::from(document::Document::default());
                }
                assert!(editor.current_tool().is_none());
                assert_eq!(dropped.get(), 0);
            })
            .unwrap();
        assert!(editor.current_tool().is_none());
        assert_eq!(dropped.get(), 1);
        assert_eq!(actions.get(), 0);
    }
}

#[test]
fn ending_crop_does_not_borrow_shared_draft_or_retain_its_state() {
    let mut editor = Editor::state();
    super::super::tests::add_ready_fixture(&mut editor);
    editor.begin_crop();
    let state = editor.document.crop_state().unwrap();
    let weak = Rc::downgrade(&state);
    let mut draft = state.borrow_mut();
    draft.draft.size = [300, 200];
    editor.end_tool();
    assert!(!editor.has_hosted_tool());
    assert!(editor.document.visit.crop.is_none());
    drop(draft);
    drop(state);
    assert!(
        weak.upgrade().is_none(),
        "no Editor field or task cycle retains the draft"
    );
}

#[test]
fn ordinary_photo_orientation_and_crop_acceptance_share_geometry_history() {
    let mut editor = Editor::state();
    super::super::tests::add_ready_fixture(&mut editor);
    let before = editor.document.visit.photo_orientation;
    editor.apply_photo_action(editing::PhotoAction::TurnRight, &UiContext::render_only());
    assert_ne!(editor.document.visit.photo_orientation, before);
    assert_eq!(
        editor.history_label(history::Direction::Undo),
        Some("Photo orientation")
    );
    let id = editor.document.visit.current_source.unwrap();
    assert_eq!(
        editor.document.project.source(id).unwrap().orientation,
        editor.document.visit.photo_orientation
    );
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let crop = Crop {
        size: [300, 200],
        ..Crop::from_region(area)
    };
    editor.accept_crop(crop).unwrap();
    assert_eq!(editor.history_label(history::Direction::Undo), Some("Crop"));
    assert_eq!(editor.document.project.source(id).unwrap().crop, Some(crop));
    assert_eq!(editor.document.visit.crop, Some(crop));
}

#[test]
#[should_panic(expected = "tool callback panic fixture")]
fn tool_callback_panics_propagate_without_recovery() {
    let mut editor = Editor::state();
    let dropped = Rc::new(Cell::new(0));
    let actions = Rc::new(Cell::new(0));
    assert!(editor.activate_tool(Placement::Photo, Box::new(Probe::new(&dropped, &actions))));
    editor.with_current_tool(|_, _| panic!("tool callback panic fixture"));
}
