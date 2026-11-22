use super::super::tests::key;
use super::*;
use crate::application::test_support::with_application;

fn document(count: usize) -> (Document, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/preview.dng");
    let paths: Vec<_> = (0..count)
        .map(|index| {
            let path = directory.path().join(format!("preview-{index}.dng"));
            std::fs::copy(&fixture, &path).unwrap();
            path
        })
        .collect();
    let mut document = Document::default();
    document.import(&paths).unwrap();
    (document, directory)
}

fn wanted(document: &Document) -> Vec<(usize, ImageKey, PathBuf)> {
    document
        .project
        .order
        .iter()
        .enumerate()
        .map(|(index, id)| {
            (
                index,
                key(document, *id),
                document.source_path(*id).unwrap(),
            )
        })
        .collect()
}

fn editor(document: Document, services: Services) -> Editor {
    let mut editor = crate::editor::tests::fixture();
    editor.document = editor.document.replacement(document);
    editor.document.thumbnails.embedded.services = Some(services);
    editor.memory_pressure = memory::Pressure::Normal;
    editor.apply_cache_budgets();
    editor.document.thumbnails.backend = Some(Arc::new(crate::source::SourceLoader::new()));
    editor
}

fn request(editor: &mut Editor, cx: &UiContext<'_, Editor>, context: &egui::Context) {
    let visible = wanted(&editor.document);
    editor.request_embedded_previews(&visible, context, cx);
}

fn controlled() -> (Services, mpsc::Receiver<Work>) {
    let (commands, requests) = mpsc::channel();
    (Services::Controlled(commands), requests)
}

fn finish(work: Work) {
    let image = load(&work.request, &|| !work.cancelled.load(Ordering::Relaxed));
    let _ = work.reply.send(image);
}

#[test]
fn visible_authored_previews_advance_and_install_without_another_paint() {
    let (document, _directory) = document(2);
    let ids = document.project.order.clone();
    let editor = editor(document, Services::Production);
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|editor, access| request(editor, access, &context));
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.embedded_preview.is_none()
        });
        app.read(|editor| {
            for id in ids {
                let key = key(&editor.document, id);
                let texture = editor
                    .document
                    .thumbnails
                    .embedded
                    .get(&key.source)
                    .unwrap();
                assert_eq!(texture.size(), [8, 16]);
            }
            assert!(editor.document.visit.canvas.ready.is_none());
            assert!(!editor.document.thumbnails.embedded.pending());
        });
    });
}

#[test]
fn composed_modal_keeps_camera_demand_and_textures_at_normal_and_narrow_sizes() {
    for width in [1100., 800.] {
        let (document, _directory) = document(2);
        let editor = editor(document, Services::Production);
        let modal = editor
            .activities
            .start(crate::editor::ActivityOptions::new("Independent foreground operation").modal());
        let context = egui::Context::default();
        with_application(editor, |app| {
            let frame =
                |app: &mut crate::application::test_support::TestApplication<'_, Editor>,
                 events: Vec<egui::Event>| {
                    let mut output = app.enter(|editor, access| {
                        context.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(width, 800.),
                                )),
                                events,
                                ..Default::default()
                            },
                            |ui| crate::platform::host::App::ui(editor, ui, access),
                        )
                    });
                    output.textures_delta.clear();
                    output
                };
            for _ in 0..4 {
                frame(app, vec![]);
                if app.read(|editor| editor.document.embedded_preview.is_some()) {
                    break;
                }
            }
            assert!(app.read(|editor| editor.document.embedded_preview.is_some()));
            app.run_until(Duration::from_secs(5), |editor| {
                editor.document.embedded_preview.is_none()
            });
            let textures: Vec<_> = app.read(|editor| {
                editor
                    .document
                    .thumbnails
                    .embedded
                    .entries
                    .iter()
                    .map(|(_, texture)| texture.texture().unwrap().id())
                    .collect()
            });
            assert_eq!(textures.len(), 2);
            let output = frame(app, vec![]);
            for texture in textures {
                assert!(
                    output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Mesh(mesh) if mesh.texture_id==texture)),
                    "visible camera textures must paint beneath disabled input"
                );
            }
            let next = || {
                [true, false]
                    .map(|pressed| egui::Event::Key {
                        key: egui::Key::ArrowRight,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                    .to_vec()
            };
            frame(app, next());
            app.read(|editor| {
                assert!(editor.interaction_blocked() && editor.workspace_input.is_blocked());
                assert!(editor.document.visit.current_source.is_none());
                assert_eq!(editor.document.active_index(), None);
                assert!(
                    editor.document.visit.canvas.ready.is_none()
                        && editor.document.visit.pending_preview.is_none()
                );
            });
            drop(modal);
            frame(app, vec![]);
            frame(app, next());
            app.read(|editor| {
                assert!(!editor.workspace_input.is_blocked());
                assert_eq!(
                    editor.document.active_index(),
                    Some(1),
                    "navigation resumes after input release"
                );
            });
        });
    }
}

#[test]
fn modal_input_preserves_current_camera_preview_and_advances_visible_demand() {
    let (document, _directory) = document(2);
    let ids = document.project.order.clone();
    let (services, requests) = controlled();
    let editor = editor(document, services);
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|editor, cx| request(editor, cx, &context));
        app.pump();
        let first = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        let modal = app.read(|editor| {
            editor.activities.start(
                crate::editor::ActivityOptions::new("Independent foreground operation").modal(),
            )
        });
        app.enter(|editor, cx| request(editor, cx, &context));
        assert!(!first.cancelled.load(Ordering::Relaxed));
        assert!(
            requests.try_recv().is_err(),
            "one preview producer owns current demand"
        );
        finish(first);
        app.run_until(Duration::from_secs(5), |editor| {
            editor
                .document
                .thumbnails
                .embedded
                .get(&key(&editor.document, ids[0]).source)
                .is_some()
        });
        app.pump();
        let second = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(second.request.key.id, ids[1]);
        assert!(!second.cancelled.load(Ordering::Relaxed));
        finish(second);
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.embedded_preview.is_none()
        });
        app.read(|editor| {
            assert!(editor.interaction_blocked() && editor.workspace_input.is_blocked());
            for id in ids {
                assert_eq!(
                    editor
                        .document
                        .thumbnails
                        .embedded
                        .get(&key(&editor.document, id).source)
                        .unwrap()
                        .size(),
                    [8, 16]
                );
            }
        });
        drop(modal);
        app.read(|editor| assert!(!editor.workspace_input.is_blocked()));
    });
}

#[test]
fn document_retirement_discards_old_results_and_queues_current_demand() {
    let (first, _first_directory) = document(1);
    let (second, _second_directory) = document(1);
    let next = wanted(&second);
    let (services, requests) = controlled();
    let editor = editor(first, services);
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|editor, access| request(editor, access, &context));
        app.pump();
        let first = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        app.enter(|editor, access| {
            editor.document = editor.document.replacement(second);
            editor.request_embedded_previews(&next, &context, access);
        });
        app.pump();
        assert!(first.cancelled.load(Ordering::Relaxed));
        let next_work = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(next_work.request.key, next[0].1.source);
        let Work { reply, .. } = first;
        assert!(
            reply
                .send(Ok(Some(egui::ColorImage::filled(
                    [1, 1],
                    egui::Color32::RED
                ))))
                .is_err(),
            "retired document has no result waiter"
        );
        finish(next_work);
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.embedded_preview.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.thumbnails.embedded.entries.len(), 1);
            assert_eq!(
                editor
                    .document
                    .thumbnails
                    .embedded
                    .get(&next[0].1.source)
                    .unwrap()
                    .size(),
                [8, 16]
            );
        });
    });
}

#[test]
fn document_camera_previews_finish_in_fullscreen_and_rewarm_when_resources_return() {
    for gpu_bytes in [0, 64 << 20] {
        let (document, _directory) = document(2);
        let paths = document
            .project
            .order
            .iter()
            .map(|id| document.source_path(*id).unwrap())
            .collect();
        let editor = crate::editor::tests::authored_fixture(&[]);
        let mut editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(30),
            |editor, cx| editor.add_raws(paths, cx),
        );
        let wanted = wanted(&editor.document);
        let (services, requests) = controlled();
        editor.document.thumbnails.embedded.services = Some(services);
        editor.document.thumbnails.backend = Some(Arc::new(crate::source::SourceLoader::new()));
        editor.set_cache_limits(cache_limits::CacheLimits {
            cpu_bytes: 0,
            gpu_bytes,
        });
        editor.memory_pressure = memory::Pressure::Normal;
        editor.apply_cache_budgets();
        let context = egui::Context::default();
        with_application(editor, |app| {
            app.enter(|editor, access| {
                editor.request_embedded_previews(&wanted[..1], &context, access)
            });
            app.pump();
            let first = requests.recv_timeout(Duration::from_secs(5)).unwrap();
            app.enter(|editor, access| {
                editor.toggle_fullscreen(&context);
                assert!(editor.fullscreen.is_some());
                let mut output =
                    context.run_ui(Default::default(), |ui| editor.hidden_filmstrip(ui, access));
                output.textures_delta.clear();
            });
            assert!(
                !first.cancelled.load(Ordering::Relaxed),
                "hidden is not cancelled"
            );
            finish(first);
            app.run_until(Duration::from_secs(5), |editor| {
                editor.document.thumbnails.embedded.entries.len() == 1
            });
            let second = requests.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(second.request.key, wanted[1].1.source);
            assert!(!second.cancelled.load(Ordering::Relaxed));
            app.read(|editor| {
                assert!(
                    editor.document.thumbnails.embedded.visible.is_empty(),
                    "second photo was never visible or requested by a card"
                )
            });
            finish(second);
            app.run_until(Duration::from_secs(5), |editor| {
                editor.document.embedded_preview.is_none()
            });
            app.read(|editor| {
                assert_eq!(editor.document.thumbnails.embedded.entries.len(), 2);
                assert!(editor.fullscreen.is_some());
                if gpu_bytes == 0 {
                    assert_eq!(editor.document.thumbnails.embedded.byte_len(), 0);
                } else {
                    for (_, image, _) in &wanted {
                        assert_eq!(
                            editor
                                .document
                                .thumbnails
                                .embedded
                                .get(&image.source)
                                .unwrap()
                                .size(),
                            [8, 16]
                        );
                    }
                }
            });
            app.pump();
            assert!(
                requests.try_recv().is_err(),
                "eviction must not restart completed offscreen work"
            );
            if gpu_bytes == 0 {
                app.enter(|editor, access| {
                    editor.set_cache_limits(cache_limits::CacheLimits {
                        cpu_bytes: 0,
                        gpu_bytes: 64 << 20,
                    });
                    editor.acquire_embedded(&context, access);
                });
                for (_, image, _) in &wanted {
                    app.pump();
                    let work = requests.recv_timeout(Duration::from_secs(5)).unwrap();
                    assert_eq!(work.request.key, image.source);
                    finish(work);
                    app.run_until(Duration::from_secs(5), |editor| {
                        editor
                            .document
                            .thumbnails
                            .embedded
                            .get(&image.source)
                            .is_some()
                    });
                }
                app.read(|editor| {
                    assert!(editor.document.thumbnails.embedded.visible.is_empty());
                    assert!(editor.fullscreen.is_some());
                    assert!(editor.document.embedded_preview.is_none());
                });
                app.pump();
                assert!(requests.try_recv().is_err());
            }
        });
    }
}

#[test]
fn application_retirement_signals_stop_and_discards_the_private_producers_result() {
    let (document, _directory) = document(1);
    let (services, requests) = controlled();
    let editor = editor(document, services);
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|editor, access| request(editor, access, &context));
        app.pump();
        let work = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        app.close_execution();
        assert!(work.cancelled.load(Ordering::Relaxed));
        assert!(work.reply.send(Ok(None)).is_err());
        app.read(|editor| assert!(editor.document.thumbnails.embedded.entries.is_empty()));
    });
}

#[test]
fn closed_worker_acknowledges_current_demand_without_paint_or_retries() {
    let (document, _directory) = document(1);
    let (services, requests) = controlled();
    let editor = editor(document, services);
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|editor, access| request(editor, access, &context));
        app.pump();
        let work = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(work);
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.embedded_preview.is_none()
        });
        app.read(|editor| {
            assert!(editor.document.thumbnails.embedded.services.is_none());
            assert_eq!(editor.document.thumbnails.embedded.entries.len(), 1);
            assert!(matches!(
                editor.document.thumbnails.embedded.entries[0].1,
                CameraImage::Unavailable
            ));
        });
        app.enter(|editor, access| request(editor, access, &context));
        app.pump();
        assert!(requests.try_recv().is_err());
    });
}

#[test]
fn cancelled_preview_is_not_memoised_as_missing_and_can_be_requested_again() {
    let (document, _directory) = document(1);
    let (services, requests) = controlled();
    let editor = editor(document, services);
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|editor, access| request(editor, access, &context));
        app.pump();
        let work = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        app.update(|editor| {
            editor.memory_pressure = memory::Pressure::Critical;
            editor.apply_cache_budgets();
        });
        assert!(work.cancelled.load(Ordering::Relaxed));
        finish(work);
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.embedded_preview.is_none()
        });
        app.read(|editor| assert!(editor.document.thumbnails.embedded.entries.is_empty()));
        app.enter(|editor, access| request(editor, access, &context));
        app.pump();
        app.read(|editor| assert!(editor.document.embedded_preview.is_none()));
        assert!(
            requests.try_recv().is_err(),
            "critical pressure prevents a new request"
        );
        app.enter(|editor, access| {
            editor.memory_pressure = memory::Pressure::Normal;
            editor.apply_cache_budgets();
            request(editor, access, &context);
        });
        app.pump();
        finish(requests.recv_timeout(Duration::from_secs(5)).unwrap());
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.embedded_preview.is_none()
        });
        app.read(|editor| {
            assert_eq!(
                editor.document.thumbnails.embedded.entries[0]
                    .1
                    .texture()
                    .unwrap()
                    .size(),
                [8, 16]
            )
        });
    });
}
