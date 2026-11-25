thread_local! {
    static EVALUATION_BUFFER_CREATIONS: std::cell::Cell<[u64; 2]> = const { std::cell::Cell::new([0, 0]) };
}

pub(crate) fn evaluation_buffer_created(bytes: u64) {
    EVALUATION_BUFFER_CREATIONS.with(|count| {
        let [buffers, total_bytes] = count.get();
        count.set([buffers + 1, total_bytes + bytes]);
    });
}

pub(crate) fn take_evaluation_buffer_creations() -> [u64; 2] {
    EVALUATION_BUFFER_CREATIONS.with(|count| count.replace([0, 0]))
}

// Exercise worker production even when this image could be recorded.
pub(crate) fn produce_image(
    context: &super::Context,
    queue: &crate::gpu::submission::Queue,
    request: super::Evaluation,
) -> super::Image<super::ImageLayout> {
    let work = context.prepare_image(request).unwrap();
    context.produce_prepared(queue, work, || false).unwrap()
}
