use super::*;

#[test]
fn original_units_cover_demand_once_and_fetch_neighbours_beyond_reconstruction_edges() {
    let source: SourceRegion = [100, 80, 40, 30].into();
    let output: SourceRegion = [108, 88, 24, 16].into();
    let pipeline = PixelPipeline {
        stages: vec![PixelStage::Gaussian { sigma: 1. }],
    };
    let units: Vec<_> = OriginalBatch::new(source, output, &pipeline, [10, 7])
        .unwrap()
        .units()
        .map(Result::unwrap)
        .collect();
    assert_eq!(units.len(), 9);
    assert_eq!(
        units[0].plan.source_read(),
        SourceRead::Originals([104, 84, 18, 15].into())
    );
    assert_eq!(units[0].offset, [4, 4]);
    assert_eq!(units[8].output, [128, 102, 4, 2].into());
    let mut coverage = [0; 24 * 16];
    for unit in units {
        for y in unit.output.y..unit.output.y + unit.output.height {
            for x in unit.output.x..unit.output.x + unit.output.width {
                coverage[((y - output.y) * output.width + x - output.x) as usize] += 1;
            }
        }
        assert_eq!(unit.plan.pitch(), [1.; 2]);
    }
    assert!(coverage.iter().all(|&count| count == 1));
    let edge = OriginalBatch::new(source, source, &pipeline, [10, 7])
        .unwrap()
        .units()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        edge.plan.source_read(),
        SourceRead::Originals([100, 80, 14, 11].into())
    );
    assert_eq!(edge.offset, [0, 0]);
}

#[test]
fn original_batch_captures_stage_values_and_rejects_nonprogressing_or_exterior_demands() {
    let source: SourceRegion = [100, 80, 40, 30].into();
    let mut pipeline = PixelPipeline {
        stages: vec![PixelStage::Gaussian { sigma: 1. }],
    };
    let batch = OriginalBatch::new(source, [112, 92, 2, 2].into(), &pipeline, [10, 7]).unwrap();
    pipeline.stages.clear();
    let unit = batch.units().next().unwrap().unwrap();
    assert_eq!(unit.offset, [4, 4]);
    assert_eq!(
        unit.plan.source_read(),
        SourceRead::Originals([108, 88, 10, 10].into())
    );
    assert!(OriginalBatch::new(source, source, &pipeline, [0, 7]).is_err());
    assert!(OriginalBatch::new(source, [99, 80, 10, 10].into(), &pipeline, [10, 7]).is_err());
}
