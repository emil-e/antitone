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

fn wanted(document: &Document) -> Vec<(usize, ImageKey, PathBuf, Source)> {
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
                document.project.source(*id).unwrap().clone(),
            )
        })
        .collect()
}

fn editor(document: Document, services: Services) -> Editor {
    let mut editor = crate::editor::tests::fixture();
    editor.document = document.into();
    editor.thumbnails.embedded.services = Some(services);
    editor
        .thumbnails
        .bind_sources(editor.document.cache().clone());
    editor
}

fn request(editor: &mut Editor, cx: &UiContext<'_, Editor>, context: &egui::Context) {
    let visible = wanted(&editor.document);
    editor.request_embedded_previews(&visible, true, context, cx);
}

fn controlled() -> (Services, mpsc::Receiver<Work>) {
    let (commands, requests) = mpsc::channel();
    (
        Services {
            commands,
            admission: Arc::new(async_lock::Semaphore::new(1)),
        },
        requests,
    )
}

fn finish(work: Work) {
    let image = load(&work.request, &|| !work.cancelled.load(Ordering::Relaxed));
    let _ = work.reply.send(image);
}

#[test]
fn visible_authored_previews_advance_and_install_without_another_paint() {
    let (document, _directory) = document(2);
    let ids = document.project.order.clone();
    let editor = editor(document, Services::new());
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|editor, access| request(editor, access, &context));
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.embedded_preview.is_none()
        });
        app.read(|editor| {
            for id in ids {
                let key = key(&editor.document, id);
                let texture = editor.thumbnails.embedded.get(&key.source).unwrap();
                assert_eq!(texture.size(), [8, 16]);
            }
            assert!(editor.document.visit.ready.is_none());
            assert!(!editor.thumbnails.embedded.pending());
        });
    });
}

#[test]
fn composed_editor_registers_visible_demand_then_uses_completed_camera_textures() {
    let (document, _directory) = document(2);
    let editor = editor(document, Services::new());
    let context = egui::Context::default();
    with_application(editor, |app| {
        for _ in 0..4 {
            app.enter(|editor, access| {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100., 800.),
                        )),
                        ..Default::default()
                    },
                    |ui| crate::platform::host::App::ui(editor, ui, access),
                );
                output.textures_delta.clear();
            });
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
                .thumbnails
                .embedded
                .entries
                .iter()
                .map(|(_, texture)| texture.as_ref().unwrap().id())
                .collect()
        });
        assert_eq!(textures.len(), 2);
        let mut output = app.enter(|editor, access| {
            context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., 800.),
                    )),
                    ..Default::default()
                },
                |ui| crate::platform::host::App::ui(editor, ui, access),
            )
        });
        output.textures_delta.clear();
        for texture in textures {
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.texture_id == texture)), "filmstrip did not use the completed camera texture");
        }
        app.read(|editor| {
            assert!(editor.document.visit.ready.is_none() && editor.pending_preview.is_none())
        });
    });
}

#[test]
fn document_retirement_drops_publication_but_private_producer_retains_serial_admission() {
    let (first, _first_directory) = document(1);
    let (second, _second_directory) = document(1);
    let next = wanted(&second);
    let (services, requests) = controlled();
    let admission = services.admission.clone();
    let editor = editor(first, services);
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|editor, access| request(editor, access, &context));
        app.pump();
        let first = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        app.enter(|editor, access| {
            editor.document = second.into();
            editor
                .thumbnails
                .bind_sources(editor.document.cache().clone());
            editor.request_embedded_previews(&next, true, &context, access);
        });
        app.pump();
        assert!(first.cancelled.load(Ordering::Relaxed));
        assert!(admission.try_acquire().is_none());
        assert!(
            requests.try_recv().is_err(),
            "replacement awaits the producer-owned permit"
        );
        let Work {
            request: _,
            cancelled: _,
            reply,
            _admission,
        } = first;
        assert!(
            reply
                .send(Ok(Some(egui::ColorImage::filled(
                    [1, 1],
                    egui::Color32::RED
                ))))
                .is_err(),
            "retired document has no result waiter"
        );
        drop(_admission);
        app.pump();
        let next_work = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(next_work.request.key, next[0].1.source);
        finish(next_work);
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.embedded_preview.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.thumbnails.embedded.entries.len(), 1);
            assert_eq!(
                editor
                    .thumbnails
                    .embedded
                    .get(&next[0].1.source)
                    .unwrap()
                    .size(),
                [8, 16]
            );
        });
        assert!(admission.try_acquire().is_some());
    });
}

#[test]
fn application_retirement_signals_stop_without_releasing_private_producer_admission() {
    let (document, _directory) = document(1);
    let (services, requests) = controlled();
    let admission = services.admission.clone();
    let editor = editor(document, services);
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|editor, access| request(editor, access, &context));
        app.pump();
        let work = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        app.close_execution();
        assert!(work.cancelled.load(Ordering::Relaxed));
        assert!(admission.try_acquire().is_none());
        let Work {
            request: _,
            cancelled: _,
            reply,
            _admission,
        } = work;
        assert!(reply.send(Ok(None)).is_err());
        drop(_admission);
        assert!(admission.try_acquire().is_some());
        app.read(|editor| assert!(editor.thumbnails.embedded.entries.is_empty()));
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
            assert!(editor.thumbnails.embedded.services.is_none());
            assert_eq!(editor.thumbnails.embedded.entries.len(), 1);
            assert!(editor.thumbnails.embedded.entries[0].1.is_none());
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
            editor.thumbnails.apply_budgets(memory::CacheBudgets {
                images: 0,
                sources: 0,
                pressure: memory::Pressure::Critical,
            })
        });
        assert!(work.cancelled.load(Ordering::Relaxed));
        finish(work);
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.embedded_preview.is_none()
        });
        app.read(|editor| assert!(editor.thumbnails.embedded.entries.is_empty()));
        app.enter(|editor, access| {
            editor.thumbnails.apply_budgets(memory::CacheBudgets::new(
                editor.thumbnails.limits,
                memory::Pressure::Normal,
            ));
            request(editor, access, &context);
        });
        app.pump();
        finish(requests.recv_timeout(Duration::from_secs(5)).unwrap());
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.embedded_preview.is_none()
        });
        app.read(|editor| {
            assert_eq!(
                editor.thumbnails.embedded.entries[0]
                    .1
                    .as_ref()
                    .unwrap()
                    .size(),
                [8, 16]
            )
        });
    });
}
