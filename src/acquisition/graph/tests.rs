use super::*;
use crate::{
    geometry::{Composition, FramePoint},
    gpu::evaluation::{
        Budget, CellRegion, CpuImage, CpuKernel, Domain, Operation, Read, Value, WorkUnits, value,
    },
    pixel_pipeline::{EffectDefinition, PixelStage},
};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
struct Declared {
    before: [u32; 2],
    after: [u32; 2],
    natural: Option<CellRegion>,
    preferred: Option<f64>,
}
impl EffectDefinition for Declared {
    fn view_pitch(&self) -> Option<f64> {
        self.preferred
    }
    fn compose(&self, input: Value) -> Result<Value, Error> {
        Ok(Value::from_operation(
            input.domain(),
            [input.clone()],
            DeclaredOperation {
                definition: self.clone(),
                domain: input.domain(),
            },
        ))
    }
}
struct DeclaredOperation {
    definition: Declared,
    domain: Domain,
}
impl Operation for DeclaredOperation {
    fn reads(&self, _: CellRegion) -> Result<Vec<Read>, Error> {
        panic!("dependency queries must use the unit's reads")
    }
    fn work_units(&self, output: CellRegion, _: Budget) -> Result<WorkUnits, Error> {
        Ok(Box::new(std::iter::once(
            self.definition.natural.unwrap_or(output),
        )))
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let mut low = [
            i64::from(output.x) - i64::from(self.definition.before[0]),
            i64::from(output.y) - i64::from(self.definition.before[1]),
        ];
        let mut high = [
            i64::from(output.right()) + i64::from(self.definition.after[0]),
            i64::from(output.bottom()) + i64::from(self.definition.after[1]),
        ];
        if let Some(bounds) = self.domain.bounds {
            for axis in 0..2 {
                low[axis] = low[axis].max(i64::from([bounds.x, bounds.y][axis]));
                high[axis] = high[axis].min(i64::from([bounds.right(), bounds.bottom()][axis]));
            }
        }
        let read = CellRegion::new(
            low.map(|v| i32::try_from(v).unwrap()),
            std::array::from_fn(|i| u32::try_from(high[i] - low[i]).unwrap()),
        )
        .validate()?;
        Ok(Some(value::Calculation::new(
            domain,
            output,
            vec![value::Calculation::input(inputs[0].request(read)?)],
            value::Operation::cpu(
                "asymmetric acquisition description",
                self.definition.clone(),
                Arc::new(NoExecution),
            ),
        )?))
    }
}
struct NoExecution;
impl CpuKernel for NoExecution {
    fn run(
        &self,
        _: &[CpuImage],
        _: CellRegion,
        _: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        panic!("acquisition description must not execute pixels")
    }
}
fn definition(before: [u32; 2], after: [u32; 2]) -> Declared {
    Declared {
        before,
        after,
        natural: None,
        preferred: None,
    }
}
fn pipeline(definition: Declared) -> PixelPipeline {
    PixelPipeline {
        stages: vec![PixelStage::new(definition)],
    }
}
fn frame() -> FrameRaster {
    FrameRaster::new(
        Composition::new(
            [100, 80, 400, 300].into(),
            Default::default(),
            Default::default(),
        )
        .unwrap(),
        FramePoint([10., 20.]),
        FramePoint([18., 26.]),
        [8, 6],
    )
    .unwrap()
}
fn plan(definition: Declared) -> Plan {
    Plan::new(Request {
        output: frame(),
        stages: &pipeline(definition),
        sampling: Sampling::NeutralFootprints,
    })
    .unwrap()
}

#[test]
fn asymmetric_unit_reads_determine_acquisition_without_running_pixels() {
    let plan = plan(definition([2, 1], [5, 3]));
    assert_eq!(plan.input().size(), [15, 10]);
    assert_eq!(plan.output_offset(), Some([2, 1]));
    assert_eq!(
        plan.source_read(),
        SourceRead::Originals([108, 99, 15, 10].into())
    );
    let input = plan.processing_frame().unwrap();
    for point in [[0.5, 0.5], [7.5, 5.5]] {
        assert_eq!(
            frame().source_point(point),
            input.source_point([point[0] + 2., point[1] + 1.])
        );
    }
    assert_eq!(plan.pitch(), [1., 1.]);
}

#[test]
fn original_envelopes_follow_asymmetric_and_natural_unit_reads() {
    let plan = |stage| {
        Plan::new(Request {
            output: frame(),
            stages: &pipeline(stage),
            sampling: Sampling::Originals {
                reconstruction: SourceSupport([1; 2]),
            },
        })
        .unwrap()
    };
    let asymmetric = plan(definition([2, 1], [5, 3]));
    assert_eq!(
        asymmetric.source_read(),
        SourceRead::Originals([107, 98, 17, 12].into())
    );
    assert_eq!(
        asymmetric.valid_output(),
        ValidOutput::Reconstruction {
            affine: [[3., 2.], [1., 0.], [0., 1.]]
        }
    );
    let mut natural = definition([1, 2], [3, 1]);
    natural.natural = Some(CellRegion::new([106, 97], [20, 15]));
    assert_eq!(
        plan(natural).source_read(),
        SourceRead::Originals([105, 95, 24, 18].into())
    );
}

#[test]
fn acquisition_reuse_checks_both_ends_of_asymmetric_input_windows() {
    let left = plan(definition([5, 0], [1, 0]));
    let right = plan(definition([1, 0], [5, 0]));
    assert!(
        !left.can_supply(&right),
        "a larger left margin does not supply a missing right neighbour"
    );
    assert!(!right.can_supply(&left));
    assert!(right.using_input(&left).is_err());
    let both = plan(definition([5, 0], [5, 0]));
    assert!(both.can_supply(&left) && both.can_supply(&right));
    let reused = right.using_input(&both).unwrap();
    assert_eq!(reused.input(), both.input());
    assert_eq!(reused.output_offset(), Some([5, 0]));
    assert_eq!(reused.pipeline(), right.pipeline());
}

#[test]
fn natural_unit_expansion_is_part_of_acquisition_demand() {
    let mut declared = definition([1, 2], [3, 1]);
    declared.natural = Some(CellRegion::new([-4, -3], [16, 12]));
    let plan = plan(declared);
    assert_eq!(plan.output_offset(), Some([5, 5]));
    assert_eq!(plan.input().size(), [20, 15]);
    assert_eq!(
        plan.source_read(),
        SourceRead::Originals([105, 95, 20, 15].into())
    );
}

#[test]
fn only_full_camera_cell_domains_clip_declared_neighbours() {
    let composition = frame().composition;
    let frame = FrameRaster {
        output: [10, 10],
        ..FrameRaster::full(composition).unwrap()
    };
    let plan = Plan::new(Request {
        output: frame,
        stages: &pipeline(definition([7, 8], [9, 10])),
        sampling: Sampling::CameraCells {
            region: composition.area(),
            size: [10, 10],
        },
    })
    .unwrap();
    assert_eq!(plan.input(), ProcessingRaster::Frame(frame));
    assert_eq!(plan.output_offset(), Some([0, 0]));
}

#[derive(Debug, PartialEq)]
struct Split;
impl EffectDefinition for Split {
    fn compose(&self, input: Value) -> Result<Value, Error> {
        let left = definition([2, 0], [0, 0]).compose(input.clone())?;
        let right = definition([0, 0], [5, 0]).compose(input.clone())?;
        Ok(Value::from_operation(input.domain(), [left, right], Merge))
    }
}
struct Merge;
impl Operation for Merge {
    fn calculation(
        self: Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        Ok(Some(value::Calculation::new(
            domain,
            output,
            inputs
                .iter()
                .map(|input| input.request(output).map(value::Calculation::input))
                .collect::<Result<_, _>>()?,
            value::Operation::cpu(
                "composed acquisition dependencies",
                (),
                Arc::new(NoExecution),
            ),
        )?))
    }
}

#[test]
fn acquisition_follows_both_parents_of_a_composed_effect_dag() {
    let pipeline = PixelPipeline {
        stages: vec![PixelStage::new(Split)],
    };
    let plan = Plan::new(Request {
        output: frame(),
        stages: &pipeline,
        sampling: Sampling::NeutralFootprints,
    })
    .unwrap();
    assert_eq!(plan.input().size(), [15, 6]);
    assert_eq!(plan.output_offset(), Some([2, 0]));
    assert_eq!(
        plan.source_read(),
        SourceRead::Originals([108, 100, 15, 6].into())
    );
}

#[test]
fn quality_refinement_counts_graph_declared_inputs_not_just_visible_cells() {
    let composition = Composition::new(
        [100, 80, 6000, 6000].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let frame = FrameRaster {
        output: [64, 64],
        ..FrameRaster::full(composition).unwrap()
    };
    let mut declared = definition([0, 0], [3000, 3000]);
    declared.preferred = Some(1.);
    let sampled = crate::pixel_pipeline::sampling::ViewSampling::new(
        frame,
        &pipeline(declared),
        Sampling::NeutralFootprints,
    )
    .unwrap();
    assert!(
        sampled.ideal[0] > frame.output[0],
        "control: quality asks for finer sampling"
    );
    assert!(sampled.limited());
    let size = sampled.plan.input().size();
    assert!(
        u64::from(size[0]) * u64::from(size[1]) <= 3064 * 3064,
        "expanded base is already larger than the refinement allowance; refinement must count all declared neighbours"
    );
    assert_eq!(sampled.presentation, frame);
}
