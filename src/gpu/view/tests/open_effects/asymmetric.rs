use super::*;
use crate::{
    frame_raster::FrameRaster,
    gpu::{
        evaluation::*,
        view::{RenderedView, ViewKernels, ViewSource, request::SourceRequest},
    },
    pixel_pipeline::EffectDefinition,
};

#[derive(Debug, PartialEq)]
struct ShiftBlend;
impl EffectDefinition for ShiftBlend {
    fn compose(&self, input: Value) -> Result<Value, crate::Error> {
        Ok(Value::from_operation(
            input.domain(),
            [input.clone()],
            Shift {
                domain: input.domain(),
            },
        ))
    }
}
struct Shift {
    domain: Domain,
}
impl Operation for Shift {
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, crate::Error> {
        let mut read = CellRegion {
            width: output.width + 3,
            height: output.height + 2,
            ..output
        };
        if let Some(bounds) = self.domain.bounds {
            read.width = read.width.min((bounds.right() - read.x) as u32);
            read.height = read.height.min((bounds.bottom() - read.y) as u32);
        }
        Ok(vec![Read::new(0, read.validate()?)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let read = self.reads(output)?[0].region;
        let bounds = self.domain.bounds.unwrap_or(read);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(read)?],
            value::Operation::new(
                "asymmetric shift fixture",
                (),
                r#"
struct Parameters { bounds:vec4i }
fn blend(p:vec2i,position:vec2f,parameters:Parameters)->vec4f{
    let center=input_0(p);
    let shifted=input_0(clamp(p+vec2i(3,2),parameters.bounds.xy,parameters.bounds.zw-vec2i(1)));
    return vec4f(center.rgb*0.25+shifted.rgb*0.75,center.w);
}
"#,
                "blend",
                value::Access::Neighborhood,
            )
            .with_uniform(
                "Parameters",
                &[bounds.x, bounds.y, bounds.right(), bounds.bottom()],
            ),
        )
        .map(Some)
    }
}

#[test]
fn asymmetric_frame_reads_acquire_real_neighbours_and_preserve_output_addressing_in_rendered_views()
{
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let kernels = pollster::block_on(ViewKernels::new(&device)).unwrap();
    let area = [100, 80, 400, 300].into();
    let frame = FrameRaster::new(
        Composition::new(area, Default::default(), Default::default()).unwrap(),
        FramePoint([10., 20.]),
        FramePoint([26., 32.]),
        [8, 6],
    )
    .unwrap();
    let input = AuthoredPhotograph::new(
        &device.limits(),
        InputSettings::Digital,
        Default::default(),
        AdjustmentLayerSnapshot::new(vec![]).unwrap(),
        DMat3::IDENTITY,
        area,
    )
    .unwrap()
    .with_pixel_pipeline(PixelPipeline {
        stages: vec![PixelStage::new(ShiftBlend)],
    });
    let make = |acquire_neighbours: bool| -> Result<Vec<[f32; 4]>, crate::Error> {
        let spec = SourceRequest::new(frame, InputSettings::Digital, DMat3::IDENTITY, None)?;
        let spec = if acquire_neighbours {
            spec.with_pipeline(input.pixel_pipeline())?
        } else {
            spec
        };
        let sampled = spec.input_frame()?;
        if acquire_neighbours {
            assert_eq!(sampled.output, [11, 8]);
            assert_eq!(
                spec.plan.output_offset(),
                Some([0, 0]),
                "all extra neighbours are on the right/bottom"
            );
        }
        let values = (0..sampled.output[1])
            .flat_map(|y| {
                (0..sampled.output[0]).map(move |x| {
                    let p = sampled
                        .source_point([f64::from(x) + 0.5, f64::from(y) + 0.5])
                        .0;
                    [
                        (p[0] * 0.01) as f32,
                        (p[1] * -0.02) as f32,
                        ((p[0] - p[1]) * 0.03) as f32,
                        8.,
                    ]
                })
            })
            .collect::<Vec<_>>();
        let source = Arc::new(ViewSource::from_neutral_samples(
            spec,
            renderer.clone(),
            kernels.clone(),
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("asymmetric frame samples"),
                contents: bytemuck::cast_slice(&values),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            }),
        ));
        let view = RenderedView::new(&device, source, &input, OutputTarget::Srgb, None)?;
        view.ensure_submitted(
            &device,
            &queue,
            &SubmissionGate::default(),
            &egui::Context::default(),
        );
        Ok(read_buffer(
            &device,
            &queue,
            &view.graded.as_ref().unwrap().pixels,
        ))
    };
    let actual = make(true).unwrap();
    assert_eq!(actual.len(), 8 * 6);
    for (i, pixel) in actual.iter().enumerate() {
        let x = 111. + 2. * (i % 8) as f64;
        let y = 101. + 2. * (i / 8) as f64;
        let expected = [
            x * 0.01 + 0.045,
            y * -0.02 - 0.06,
            (x - y) * 0.03 + 0.045,
            8.,
        ];
        for (a, b) in pixel.iter().zip(expected) {
            assert!(
                (f64::from(*a) - b).abs() < 2e-6,
                "pixel={i} actual={pixel:?} expected={expected:?}"
            );
        }
    }
    assert!(
        (f64::from(actual[0][0]) - 1.11).abs() > 0.04,
        "control: using only the center is observably different"
    );
    assert!(
        matches!(make(false), Err(crate::Error::InvalidParameter(_))),
        "missing far-side neighbours must reject rather than become viewport edges"
    );
}
