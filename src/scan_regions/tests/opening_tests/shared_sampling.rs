use super::*;
use crate::{
    project::{Development, SourceDevelopment, SourceId},
    scan_regions::{Evidence, FrameInput, prepare_spatial},
    spatial_metadata::{FileRevision, Input},
};

#[test]
fn shared_preparation_publishes_holder_safe_support_and_keeps_a_nonrecurring_dark_scene() {
    let directory = tempfile::tempdir().unwrap();
    for angle in [0., 1.25, -1.25] {
        let captures: Vec<_> = (0..3).map(|i| capture(i, angle)).collect();
        let frames = captures
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let path = directory.path().join(format!("{i}.raw"));
                std::fs::write(&path, [i as u8; 16]).unwrap();
                let input = Input {
                    source: SourceId(i as u64 + 1),
                    revision: FileRevision::read(&path).unwrap(),
                    correction: None,
                    development: SourceDevelopment {
                        decode: Development::new("test-backend".into(), [1.; 4]),
                        dimensions: [800, 600],
                        active_area: [12, 20, 768, 512],
                        camera_model: "opening fixture".into(),
                        camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    },
                };
                FrameInput::new(
                    input,
                    c.size,
                    c.rgb.clone(),
                    Evidence::observe(c.size, &c.rgb).unwrap(),
                )
                .unwrap()
            })
            .collect();
        let metadata = prepare_spatial(
            frames,
            Development::new("test-backend".into(), [1.; 4]),
            || Ok(()),
        )
        .unwrap();
        assert_eq!(metadata.groups[0].sources.len(), 3);
        let mut checked_holder = 0;
        for (c, source) in captures.iter().zip(&metadata.groups[0].sources) {
            let areas = source.sampling_areas();
            assert!(
                areas.ready(),
                "positive source needs usable suggestions: {angle} {areas:?}"
            );
            assert!(areas.opening_shared);
            let footprint = |x, y| SourceRegion {
                x: c.area.x + x * 4,
                y: c.area.y + y * 4,
                width: 8,
                height: 8,
            };
            assert!(areas.contains_footprint(footprint(96, 64), c.area, true));
            // Each supplied camera cell represents a constant4x4 source block.
            // The unchanged-holder capture defines the mask independently of detector output.
            for y in 0..127usize {
                for x in 0..191usize {
                    if (0..2).any(|dy| {
                        (0..2).any(|dx| captures[1].rgb[(y + dy) * 192 + x + dx] == [10.; 3])
                    }) {
                        assert!(
                            !areas.contains_footprint(footprint(x as u32, y as u32), c.area, false),
                            "known holder admitted at{x},{y}, angle{angle}"
                        );
                        checked_holder += 1;
                    }
                }
            }
        }
        assert!(
            checked_holder > 1000,
            "contrasting holder mask must not be empty"
        );
        if angle == 0. {
            let c = &captures[0];
            assert_eq!(c.rgb[64 * 192 + 45], [10.; 3]);
            assert!(
                metadata.groups[0].sources[0]
                    .sampling_areas()
                    .contains_footprint(
                        SourceRegion {
                            x: c.area.x + 45 * 4,
                            y: c.area.y + 64 * 4,
                            width: 8,
                            height: 8
                        },
                        c.area,
                        false,
                    ),
                "dark subject detail is not recurring holder material"
            );
        }
    }
}
