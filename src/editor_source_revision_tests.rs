use super::*;
use crate::project::Save;

#[test]
fn authored_raw_byte_edits_reopen_without_identity_rejection_and_old_seed_keeps_its_revision() {
    use std::io::Write;
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderers = crate::renderer::RendererCache::default();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("edited.dng");
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
        std::fs::copy(fixture, &path).unwrap();
        let mut document = Document::default();
        document.import(std::slice::from_ref(&path)).unwrap();
        let id = document.project.order[0];
        let source = document.project.source(id).unwrap();
        let wb = document.project.development.as_ref().unwrap().fixed_wb;
        let (before, _) = prepare_source(
            &renderers,
            &device,
            &queue,
            path.clone(),
            Some(wb),
            Some(source),
            None,
            document.project.camera_basis.as_ref(),
        )
        .unwrap();
        let saved = document
            .save(&directory.path().join("project.json"), Save::New)
            .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"intentional RAW annotation")
            .unwrap();
        let reopened = Document::open(saved.path.as_ref().unwrap()).unwrap();
        assert_eq!(reopened.project, saved.project);
        assert!(!reopened.dirty());
        let (file, proxy) = before.thumbnail.unwrap();
        let current = SourceKey::new(source, &before.development, FileKey::new(&path));
        assert_ne!(file, current.file);
        let mut thumbnails = Thumbnails::default();
        thumbnails.seed(source, &before.development, file, proxy.clone());
        assert!(thumbnails.sources.lock().unwrap().get(&current).is_none());
        let (after, _) = prepare_source(
            &renderers,
            &device,
            &queue,
            path,
            Some(wb),
            Some(source),
            None,
            document.project.camera_basis.as_ref(),
        )
        .unwrap();
        assert_eq!(after.thumbnail.unwrap().1.pixels, proxy.pixels);
    });
}
