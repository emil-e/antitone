use super::*;

#[test]
fn source_and_masked_sink_share_the_same_scratch_allowance() {
    const PAYLOAD: usize = 2 << 20;
    const BUDGET: u64 = (4 << 20) + 4096;

    struct Payload;
    impl Operation for Payload {
        fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
            Ok(Some(region))
        }
        fn lower(
            &self,
            fragment: &mut Fragment,
            input: Option<PlannedImage>,
            output: PlannedImage,
        ) -> Result<(), Error> {
            let payload = fragment.resource(Storage::Data(vec![0; PAYLOAD].into()));
            fragment.compute(
                r#"
@group(0) @binding(0) var<storage, read> input: array<vec4f>;
@group(0) @binding(1) var<storage, read_write> output: array<vec4f>;
@group(0) @binding(2) var<storage, read> payload: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x < arrayLength(&output) {
        output[id.x] = input[id.x] + vec4f(payload[0]);
    }
}
"#,
                "main",
                vec![
                    (0, Binding::Read(input.unwrap().resource)),
                    (1, Binding::Write(output.resource)),
                    (2, Binding::Read(payload)),
                ],
                output.cells(),
            );
            Ok(())
        }
    }

    let (input, reads) = fixture([512, 129], vec![[-2., 3., 5., 1.]; 512 * 129]);
    let input = ImageNode::new(input.domain, Some(input), Payload);
    let mapping = Mapping {
        start: [256., 0.],
        axes: [[256., 64.5], [-256., 64.5]],
    };
    let root = reconstruct_complete(input, output(mapping, [0, 0, 1, 1].into())).unwrap();
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let evaluation = context
        .request(
            root.clone(),
            Region {
                bounds: root.domain.finite().unwrap(),
                grid: root.domain.grid,
            },
            Budget {
                tile: [1, 1],
                scratch_bytes: BUDGET,
            },
        )
        .unwrap();
    let mut actual = Vec::new();
    let stats = context
        .read(
            &queue,
            &Default::default(),
            &evaluation,
            || false,
            |_, pixels| {
                actual.extend_from_slice(pixels);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(actual, vec![[-2., 3., 5., 9.]]);

    let reads = reads.lock().unwrap();
    assert!(reads.len() > 1, "exercise streamed contributions");
    assert!(stats.scratch_bytes <= BUDGET, "{stats:?}");
    for region in reads.iter() {
        let cells = u64::from(region.width) * u64::from(region.height);
        // Input/output rasters, declared payload, one bit per source cell, parent result,
        // and reduction means/flags coexist. Uniforms add further storage to this lower bound.
        let live = cells * 32 + PAYLOAD as u64 + cells.div_ceil(32) * 4 + 16 + 20;
        assert!(
            live <= BUDGET,
            "{region:?}: at least {live} live scratch bytes"
        );
        assert!(
            stats.scratch_bytes >= live,
            "{stats:?}: omitted live storage"
        );
    }
    assert_eq!(
        reads
            .iter()
            .map(|r| u64::from(r.width) * u64::from(r.height))
            .sum::<u64>(),
        512 * 129
    );
}
