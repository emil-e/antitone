use super::*;

#[test]
fn healing_normalized_dark_signed_hdr_and_repeating_texture_preserves_scene_content() {
    let size = [96, 96];
    let mask = circle(size, [48, 48], 5);
    for textured in [false, true] {
        let clean: Vec<Ap1> = (0..96)
            .flat_map(|y| {
                (0..96).map(move |x| {
                    let texture = if textured {
                        0.008 * (std::f32::consts::TAU * (x % 16) as f32 / 16.).sin()
                            + 0.006 * (std::f32::consts::TAU * (y % 16) as f32 / 16.).cos()
                    } else {
                        0.
                    };
                    [
                        0.04 + x as f32 * 0.0001 + texture,
                        -0.02 + y as f32 * 0.0002 + texture * 0.25,
                        1.1 + x as f32 * 0.0003 + texture * 2.,
                    ]
                })
            })
            .collect();
        for gain in [1. / 256., 0.25, 1., 4., 256.] {
            for automatic in [false, true] {
                let mut source: Vec<_> = clean.iter().map(|p| p.map(|v| v * gain)).collect();
                for (pixel, hidden) in source.iter_mut().zip(&mask) {
                    if *hidden {
                        *pixel = [-100. * gain, 100. * gain, 0.];
                    }
                }
                let support = SupportMask::new([0, 0], size, mask.clone()).unwrap();
                let donor = if automatic {
                    let SearchOutcome::Candidate(offset) = search_donor(
                        &support,
                        None,
                        active(size),
                        &mut |[x, y]| Ok(source[(y * size[0] + x) as usize]),
                        Instant::now() + Duration::from_secs(3),
                        SEARCH_ACCEPTABLE_SCORE,
                        &mut Execution::default(),
                    )
                    .unwrap() else {
                        panic!("normalization fixture has clean donors")
                    };
                    offset
                } else {
                    [-32, 0]
                };
                let (healed, _) = heal_ranked(&source, size, &mask, donor).unwrap();
                assert!(outside_bits_equal(&healed, &source, &mask));
                let mut checked = 0;
                for (index, hidden) in mask.iter().enumerate() {
                    if !hidden {
                        continue;
                    }
                    checked += 1;
                    for channel in 0..3 {
                        let reconstructed = healed[index][channel] / gain;
                        assert!(
                            (reconstructed - clean[index][channel]).abs() < 2e-5,
                            "texture={textured} gain={gain} donor={donor:?} pixel={index} channel={channel}: {reconstructed} vs {}",
                            clean[index][channel]
                        );
                    }
                }
                assert!(
                    checked > 50,
                    "fixture must exercise an interior and its seam"
                );
            }
        }
    }
}
