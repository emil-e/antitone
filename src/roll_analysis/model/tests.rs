use super::*;

#[test]
fn ordered_batches_keep_groups_and_local_indices_even_with_empty_groups() {
    let points = [DVec3::splat(0.03), DVec3::splat(0.04), DVec3::splat(0.05)];
    let batch = SampleBatch::new([&[][..], &points[..2], &points[2..], &[][..]]);
    let negative = NegativeInput::from(Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.005),
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    });
    let scalar = Inversion::new(negative).unwrap();
    let mut visited = Vec::new();
    assert!(
        batch
            .read(
                &mut ScalarReference,
                negative,
                &mut || false,
                |group, index, positive| {
                    visited.push((group, index, positive));
                    Ok(())
                }
            )
            .unwrap()
    );
    assert_eq!(
        visited,
        [
            (1, 0, scalar.apply(points[0]).unwrap().rgb),
            (1, 1, scalar.apply(points[1]).unwrap().rgb),
            (2, 0, scalar.apply(points[2]).unwrap().rgb)
        ]
    );
}

#[test]
fn forward_model_results_cannot_silently_omit_reorder_or_invent_samples() {
    struct ResultShape(u8);
    impl ForwardModel for ResultShape {
        fn read(
            &mut self,
            _: NegativeInput,
            _: &[DVec3],
            _: &mut dyn FnMut() -> bool,
            consume: &mut dyn FnMut(usize, DVec3) -> Result<(), Error>,
        ) -> Result<bool, Error> {
            match self.0 {
                0 => {
                    consume(0, DVec3::ONE)?;
                    Ok(true)
                }
                1 => {
                    consume(1, DVec3::ONE)?;
                    Ok(true)
                }
                2 => {
                    consume(0, DVec3::ONE)?;
                    consume(0, DVec3::ONE)?;
                    Ok(true)
                }
                3 => {
                    consume(0, DVec3::splat(f64::NAN))?;
                    Ok(true)
                }
                4 => {
                    consume(0, DVec3::ONE)?;
                    consume(1, DVec3::ONE)?;
                    consume(2, DVec3::ONE)?;
                    Ok(true)
                }
                _ => {
                    consume(0, DVec3::ONE)?;
                    Ok(false)
                }
            }
        }
    }
    let points = [DVec3::splat(0.03); 2];
    let batch = SampleBatch::new([points.as_slice()]);
    let negative = NegativeInput::from(Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.005),
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    });
    for mode in 0..5 {
        assert!(
            batch
                .read(
                    &mut ResultShape(mode),
                    negative,
                    &mut || false,
                    |_, _, _| Ok(())
                )
                .is_err()
        );
    }
    assert!(
        !batch
            .read(&mut ResultShape(5), negative, &mut || false, |_, _, _| Ok(
                ()
            ))
            .unwrap()
    );
}
