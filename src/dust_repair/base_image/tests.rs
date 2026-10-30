use super::*;

#[test]
fn region_inputs_keep_original_coordinates_and_do_not_read_masked_defects() {
    let region = SourceRegion::from([12, 9, 3, 2]);
    let needed = [true, false, true, false, true, false];
    let expected = [[12, 9], [14, 9], [13, 10]];
    let mut acquired = Vec::new();
    let pixels = read(
        &mut |point: [u32; 2]| {
            assert!(
                expected.contains(&point),
                "masked defects must not be sampled"
            );
            acquired.push(point);
            Ok([
                -(point[0] as f32),
                point[1] as f32 + 4.,
                2. * point[1] as f32,
            ])
        },
        region,
        Some(&needed),
        &mut Execution::default(),
    )
    .unwrap()
    .unwrap();

    assert_eq!(acquired, expected);
    assert_eq!(
        pixels,
        [
            [-12., 13., 18.],
            [0.; 3],
            [-14., 13., 18.],
            [0.; 3],
            [-13., 14., 20.],
            [0.; 3],
        ]
    );
}

#[test]
fn region_inputs_reject_bad_shapes_and_discard_cancelled_partial_reads() {
    struct Short {
        calls: usize,
    }

    impl BaseImage for Short {
        fn read(
            &mut self,
            _: SourceRegion,
            _: Option<&[bool]>,
            _: &dyn Fn() -> bool,
        ) -> Result<Option<Vec<Ap1>>, Error> {
            self.calls += 1;
            Ok(Some(vec![[0.; 3]]))
        }
    }

    let region = SourceRegion::from([12, 9, 3, 2]);
    let mut short = Short { calls: 0 };
    assert!(matches!(
        read(&mut short, region, None, &mut Execution::default()),
        Err(Error::InvalidParameter(_))
    ));
    assert_eq!(short.calls, 1);
    assert!(matches!(
        read(&mut short, region, Some(&[true]), &mut Execution::default()),
        Err(Error::InvalidParameter(_))
    ));
    assert_eq!(short.calls, 1);
    assert!(
        read(
            &mut short,
            region,
            None,
            &mut Execution {
                cancelled: &|| true
            }
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(short.calls, 1);

    let stopped = std::cell::Cell::new(false);
    let mut acquired = 0;
    assert!(
        read(
            &mut |_: [u32; 2]| {
                acquired += 1;
                stopped.set(true);
                Ok([1.; 3])
            },
            region,
            None,
            &mut Execution {
                cancelled: &|| stopped.get()
            }
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(acquired, 1);
}
