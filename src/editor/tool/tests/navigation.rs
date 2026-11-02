use super::*;
use std::cell::RefCell;

struct AroundCanvas {
    call_shared: bool,
    viewport: Option<Viewport>,
    trace: Rc<RefCell<Vec<(&'static str, f32)>>>,
}

impl Tool for AroundCanvas {
    fn presentation(&self, _editor: &Editor) -> Presentation {
        Presentation {
            show_viewer: true,
            identity: presentation::ToolIdentity {
                icon: icons::Icon::Pick,
                title: "Canvas probe",
            },
            primary: None,
            dismiss: "Done",
            fit_label: "Fit",
            warning: None,
            help: "",
            navigation_help: None,
        }
    }

    fn viewport<'a>(&'a mut self, editor: &'a mut Editor) -> &'a mut Viewport {
        self.viewport
            .as_mut()
            .unwrap_or(&mut editor.document.visit.canvas.viewport)
    }

    fn canvas(
        &mut self,
        canvas: &mut canvas::Canvas<'_>,
        editor: &mut Editor,
        _cx: &UiContext<'_, Editor>,
    ) -> CanvasLayout {
        let trace = self.trace.clone();
        let call_shared = self.call_shared;
        let rect = canvas.response.rect;
        let composition = canvas.composition;
        let ppp = canvas.ui.ctx().pixels_per_point();
        let relayout =
            |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
        let viewport = self.viewport(editor);
        let mut layout = canvas.layout(viewport, relayout);
        trace.borrow_mut().push(("before", layout.scale));
        if call_shared {
            layout = crate::editor::navigation::navigate_view(
                viewport,
                canvas.ui,
                canvas.response,
                layout,
                relayout,
            )
            .0;
        }
        trace.borrow_mut().push(("after", layout.scale));
        layout
    }

    fn action(&mut self, _action: Action, editor: &mut Editor) {
        editor.end_tool();
    }
}

#[test]
fn tool_composes_layout_and_wraps_or_omits_navigation_with_its_chosen_viewport() {
    for call_shared in [false, true] {
        for private_viewport in [false, true] {
            for enabled in [false, true] {
                let trace = Rc::new(RefCell::new(Vec::new()));
                let mut editor = Editor::state();
                crate::editor::tests::add_ready_fixture(&mut editor);
                editor.document.visit.canvas.viewport.zoom = Some(1.);
                let mut viewport = Viewport::default();
                viewport.zoom = Some(1.);
                assert!(editor.activate_tool(
                    Placement::Photo,
                    Box::new(AroundCanvas {
                        call_shared,
                        viewport: private_viewport.then_some(viewport),
                        trace: trace.clone(),
                    })
                ));
                crate::application::test_support::with_application(editor, |app| {
                    let context = egui::Context::default();
                    let canvas =
                        egui::Rect::from_min_size(egui::pos2(20., 20.), egui::vec2(200., 150.));
                    let render = |events| {
                        app.enter(|editor, cx| {
                            let mut result = None;
                            let mut output = context.run_ui(
                                egui::RawInput {
                                    screen_rect: Some(egui::Rect::from_min_size(
                                        egui::Pos2::ZERO,
                                        egui::vec2(400., 300.),
                                    )),
                                    events,
                                    ..Default::default()
                                },
                                |ui| {
                                    let response = ui.interact(
                                        canvas,
                                        ui.id().with("navigation"),
                                        egui::Sense::click_and_drag(),
                                    );
                                    let area =
                                        editor.document.visit.canvas.ready.as_ref().unwrap().area;
                                    let composition = editor.normal_canvas_composition(area, None);
                                    result = editor.hosted_canvas(
                                        ui,
                                        &response,
                                        composition,
                                        None,
                                        enabled,
                                        cx,
                                    );
                                },
                            );
                            output.textures_delta.clear();
                            result.unwrap()
                        })
                    };
                    render(vec![]);
                    render(vec![]);
                    trace.borrow_mut().clear();
                    let result = render(vec![
                        egui::Event::PointerMoved(canvas.center()),
                        egui::Event::Zoom(2.),
                    ]);
                    let expected = if call_shared && enabled { 2. } else { 1. };
                    assert_eq!(*trace.borrow(), [("before", 1.), ("after", expected)]);
                    assert_eq!(result.scale, expected);
                    app.update(|editor| {
                        assert_eq!(
                            editor.document.visit.canvas.viewport.zoom,
                            Some(if private_viewport { 1. } else { expected })
                        );
                        assert_eq!(
                            editor.with_tool_viewport(|viewport| viewport.zoom),
                            Some(expected)
                        );
                    });
                });
            }
        }
    }
}
