use super::*;

#[test]
fn recording_allocates_scratch_before_encoding_so_painting_cannot_fail_allocating_it() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let evaluation = request(&context, source(Arc::new(Pixels(reads.clone()))));
    let mut fragment = Fragment::default();
    let scratch = fragment.image(evaluation.image().domain(), evaluation.job.region());
    let output = fragment.image(evaluation.image().domain(), evaluation.job.region());
    fragment.compute(
        "@group(0) @binding(0) var<storage,read_write> output:array<vec4f>;
         @compute @workgroup_size(64) fn coordinates(@builtin(global_invocation_id) p:vec3u) {
         if p.x < 32u { output[p.x]=vec4f(f32(p.x%8u),f32(p.x/8u),-2.,8.); } }",
        "coordinates",
        vec![(0, Binding::Write(scratch.resource))],
        1,
    );
    fragment.compute(
        "@group(0) @binding(0) var<storage,read> input:array<vec4f>;
         @group(0) @binding(1) var<storage,read_write> output:array<vec4f>;
         @compute @workgroup_size(64) fn copy(@builtin(global_invocation_id) p:vec3u) {
         if p.x < 32u { output[p.x]=input[p.x]; } }",
        "copy",
        vec![
            (0, Binding::Read(scratch.resource)),
            (1, Binding::Write(output.resource)),
        ],
        1,
    );
    let plan = Plan::from_work(fragment, output).unwrap();
    crate::gpu::test_support::take_evaluation_buffer_creations();
    let recorded = Preparation::new(evaluation, PreparedWork::Gpu(plan))
        .record(&context)
        .unwrap();
    assert!(crate::gpu::test_support::take_evaluation_buffer_creations()[0] > 0);
    let commands = recorded.commands.as_ref().unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    commands
        .encode_result(&device, &mut encoder, &recorded.pixels)
        .unwrap();
    assert_eq!(
        crate::gpu::test_support::take_evaluation_buffer_creations(),
        [0, 0]
    );
    queue.submit([encoder.finish()]);
    commands.submitted();
    assert_eq!(
        super::super::super::super::execute::read(&device, &queue, &recorded.pixels),
        (0..32)
            .map(|i| [(i % 8) as f32, (i / 8) as f32, -2., 8.])
            .collect::<Vec<_>>()
    );
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}
