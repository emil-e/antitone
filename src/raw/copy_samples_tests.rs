use super::*;

#[test]
fn camera_rectangle_copy_preserves_source_scale_channels_and_destination_padding() {
    let codes: Vec<u16> = (0..5 * 4 * 4)
        .map(|i| ((i * 3911 + 7) % 65536) as u16)
        .collect();
    let continuous: Vec<[f32; 3]> = (0..5 * 4)
        .map(|i| {
            [
                70000.125 + i as f32,
                i as f32 * 0.25,
                0.625 + i as f32 * 19.5,
            ]
        })
        .collect();
    for floating in [false, true] {
        for gain in [1., 4. / 3.] {
            let image = Developed {
                pixels: &codes,
                continuous: floating.then_some(continuous.as_slice()),
                sample_scale: gain,
                width: 5,
                height: 4,
                maximum: 65535,
                data_maximum: 65535,
                warnings: 0,
                pre_mul: [1.; 4],
                _owner: PhantomData,
            };
            for area in [
                Rect {
                    x: 0,
                    y: 0,
                    width: 5,
                    height: 4,
                },
                Rect {
                    x: 2,
                    y: 1,
                    width: 3,
                    height: 2,
                },
                Rect {
                    x: 4,
                    y: 3,
                    width: 1,
                    height: 1,
                },
            ] {
                let stride = area.width as usize + 2;
                let mut out = vec![[-123.; 4]; stride * (area.height as usize + 1)];
                image.copy_samples(area, &mut out, stride).unwrap();
                for y in 0..area.height as usize + 1 {
                    for x in 0..stride {
                        let expected = if y < area.height as usize && x < area.width as usize {
                            let i = (area.y as usize + y) * 5 + area.x as usize + x;
                            if floating {
                                [
                                    continuous[i][0] * gain,
                                    continuous[i][1] * gain,
                                    continuous[i][2] * gain,
                                    0.,
                                ]
                            } else {
                                std::array::from_fn(|c| f32::from(codes[4 * i + c]) * gain)
                            }
                        } else {
                            [-123.; 4]
                        };
                        assert_eq!(
                            out[y * stride + x].map(f32::to_bits),
                            expected.map(f32::to_bits)
                        );
                        if y < area.height as usize && x < area.width as usize {
                            assert_eq!(
                                out[y * stride + x].map(f32::to_bits),
                                image
                                    .pixel_samples(area.x + x as u32, area.y + y as u32)
                                    .unwrap()
                                    .map(f32::to_bits)
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_camera_copy_layout_rejects_before_writing() {
    let image = Developed {
        pixels: &[1; 80],
        continuous: None,
        sample_scale: 1.,
        width: 5,
        height: 4,
        maximum: 65535,
        data_maximum: 65535,
        warnings: 0,
        pre_mul: [1.; 4],
        _owner: PhantomData,
    };
    for (area, stride, len) in [
        (
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            },
            1,
            1,
        ),
        (
            Rect {
                x: 4,
                y: 3,
                width: 2,
                height: 1,
            },
            2,
            2,
        ),
        (
            Rect {
                x: u32::MAX,
                y: 0,
                width: 1,
                height: 1,
            },
            1,
            1,
        ),
        (
            Rect {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
            },
            2,
            4,
        ),
        (
            Rect {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
            },
            4,
            7,
        ),
    ] {
        let mut out = vec![[19.; 4]; len];
        assert!(image.copy_samples(area, &mut out, stride).is_err());
        assert!(out.iter().all(|v| *v == [19.; 4]));
    }
}
