use super::*;
use wgpu::util::DeviceExt;

fn buffer(device: &wgpu::Device, value: u32) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("completion-owned test input"),
        contents: bytemuck::cast_slice(&[value; 16]),
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
    })
}
fn target(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("completion-owned test output"),
        size: 64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    })
}
fn copy(
    working: &RunWorkingSet,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gate: &crate::gpu::submission::SubmissionGate,
    source: &Arc<ImageValue>,
    destination: &wgpu::Buffer,
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source.as_buffer().unwrap(), 0, destination, 0, 64);
    let mut resources = Allocations::default();
    resources.image(source);
    resources.buffer(destination);
    working
        .submit_gpu(device, (queue, gate), encoder, resources, || {})
        .unwrap();
}
fn read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u32> {
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    receive.recv().unwrap().unwrap();
    let values = bytemuck::cast_slice(&buffer.get_mapped_range(..).unwrap()).to_vec();
    buffer.unmap();
    values
}

#[test]
fn submitted_values_survive_logical_retirement_and_release_at_completion() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let working = RunWorkingSet::default();
    let source = ImageValue::new(buffer(&device, 7));
    let weak = Arc::downgrade(&source);
    let destination = target(&device);
    copy(
        &working,
        &device,
        &queue,
        &Default::default(),
        &source,
        &destination,
    );
    drop(source);
    assert!(weak.upgrade().is_some());
    working.finish_gpu().unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(read(&device, &destination), vec![7; 16]);
}

#[test]
fn submitted_storage_is_deduplicated_without_a_separate_byte_allowance() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let working = RunWorkingSet::default();
    let source = ImageValue::new(buffer(&device, 11));
    let weak = Arc::downgrade(&source);
    let destination = target(&device);
    let gate = Default::default();
    copy(&working, &device, &queue, &gate, &source, &destination);
    copy(&working, &device, &queue, &gate, &source, &destination);
    drop(source);
    assert_eq!(working.gpu.submitted_bytes(), 128);
    assert_eq!(working.gpu.counts(), (2, 0));
    assert!(weak.upgrade().is_some());
    working.finish_gpu().unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(working.gpu.submitted_bytes(), 0);
    assert_eq!(read(&device, &destination), vec![11; 16]);
}

#[test]
fn deferred_submissions_preserve_interleaved_queue_write_order() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let working = RunWorkingSet::default();
    let source = ImageValue::new(buffer(&device, 0));
    let first = target(&device);
    let second = target(&device);
    let gate = Default::default();
    queue.write_buffer(
        source.as_buffer().unwrap(),
        0,
        bytemuck::cast_slice(&[1u32; 16]),
    );
    copy(&working, &device, &queue, &gate, &source, &first);
    queue.write_buffer(
        source.as_buffer().unwrap(),
        0,
        bytemuck::cast_slice(&[2u32; 16]),
    );
    copy(&working, &device, &queue, &gate, &source, &second);
    assert_eq!(
        working.gpu.counts(),
        (2, 0),
        "submissions using the same live working set do not wait for completion"
    );
    working.finish_gpu().unwrap();
    assert_eq!(read(&device, &first), vec![1; 16]);
    assert_eq!(read(&device, &second), vec![2; 16]);
}

#[test]
fn submission_establishes_completion_ownership_before_publication() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let working = RunWorkingSet::default();
    let source = ImageValue::new(buffer(&device, 31));
    let weak = Arc::downgrade(&source);
    let destination = target(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source.as_buffer().unwrap(), 0, &destination, 0, 64);
    let mut resources = Allocations::default();
    resources.image(&source);
    resources.buffer(&destination);
    drop(source);
    let mut published = false;
    let result = working.submit_gpu(
        &device,
        (&queue, &Default::default()),
        encoder,
        resources,
        || {
            let image = weak
                .upgrade()
                .expect("publication retains the physical source");
            assert!(
                working.gpu.owns(&image),
                "the submission owns storage before publication"
            );
            published = true;
        },
    );
    result.unwrap();
    assert!(published);
    assert_eq!(working.gpu.counts(), (1, 0));
    assert!(weak.upgrade().is_some());
    working.finish_gpu().unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(read(&device, &destination), vec![31; 16]);
}

#[test]
fn reused_storage_does_not_allow_unbounded_submission_descriptors() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let working = RunWorkingSet::default();
    let source = ImageValue::new(buffer(&device, 23));
    let destination = target(&device);
    let gate = Default::default();
    for _ in 0..96 {
        copy(&working, &device, &queue, &gate, &source, &destination);
    }
    assert_eq!(working.gpu.counts().0, 96);
    assert!(working.gpu.peak_queued() <= 32);
    working.finish_gpu().unwrap();
    assert_eq!(read(&device, &destination), vec![23; 16]);
}

#[test]
fn publication_unwinding_keeps_completion_owned_cleanup() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let source = ImageValue::new(buffer(&device, 29));
    let weak = Arc::downgrade(&source);
    let destination = target(&device);
    let gate = crate::gpu::submission::SubmissionGate::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let working = RunWorkingSet::default();
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(source.as_buffer().unwrap(), 0, &destination, 0, 64);
        let mut resources = Allocations::default();
        resources.image(&source);
        resources.buffer(&destination);
        drop(source);
        working
            .submit_gpu(&device, (&queue, &gate), encoder, resources, || {
                panic!("publication failed")
            })
            .unwrap();
    }));
    assert!(result.is_err());
    assert!(weak.upgrade().is_none());
    assert!(!gate.pending());
    assert_eq!(read(&device, &destination), vec![29; 16]);
}

#[test]
fn cancellation_cleanup_drains_submitted_storage_without_optional_retention() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let source = ImageValue::new(buffer(&device, 19));
    let weak = Arc::downgrade(&source);
    let destination = target(&device);
    let result: Result<(), Error> = {
        let working = RunWorkingSet::default();
        copy(
            &working,
            &device,
            &queue,
            &Default::default(),
            &source,
            &destination,
        );
        drop(source);
        Err(Error::Cancelled)
    };
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(weak.upgrade().is_none());
    assert_eq!(read(&device, &destination), vec![19; 16]);
}
