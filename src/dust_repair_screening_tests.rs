use super::*;
use std::cell::Cell;

fn integer_sums(input: &ScreenInput<'_>, source: [u32; 2], processed: usize) -> [u64; 5] {
    let quant = |pixel: usize, ch: usize| {
        ((input.features[pixel * 5 + ch / 2] >> (16 * (ch % 2))) as u16) as i16 as i32
    };
    let upper = if input.paired_weights {
        [0, 0]
    } else {
        input.upper_totals.map(u64::from)
    };
    let mut result = [0, 0, processed as u64, upper[0], upper[1]];
    for i in 0..input.weights.len() {
        let x = i % input.local_width as usize;
        let y = i / input.local_width as usize;
        let t =
            (input.target[1] as usize + y) * input.width as usize + input.target[0] as usize + x;
        let s = (source[1] as usize + y) * input.width as usize + source[0] as usize + x;
        let sf = input.features[s * 5 + 4] >> 16;
        let tf = input.features[t * 5 + 4] >> 16;
        if input.paired_weights && sf & tf & 1 != 0 {
            result[3] += u64::from(input.weights[i] >> 9);
        }
        if input.paired_weights && sf & tf & 2 != 0 {
            result[4] += u64::from(input.weights[i] >> 9);
        }
        if i >= processed {
            continue;
        }
        for ch in 0..9 {
            if (ch < 3 && sf & tf & 1 == 0) || (ch >= 3 && sf & tf & 2 == 0) {
                continue;
            }
            let a = quant(t, ch);
            let b = quant(s, ch);
            if a == -32768 || b == -32768 {
                continue;
            }
            let gap = ((a - b).abs() - 3).clamp(0, 4095) as u64;
            result[usize::from(ch >= 3)] += gap * gap * u64::from(input.weights[i] & 511);
        }
    }
    result
}
struct ScalarScreen {
    calls: usize,
    prefix: usize,
}
impl Screener for ScalarScreen {
    fn admit(&self, _: usize, _: usize, _: usize) -> bool {
        true
    }
    fn screen(
        &mut self,
        input: ScreenInput<'_>,
        _: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[u64; 5]>>, Error> {
        self.calls += 1;
        Ok(Some(
            input
                .candidates
                .iter()
                .map(|p| integer_sums(&input, *p, self.prefix.min(input.weights.len())))
                .collect(),
        ))
    }
}
fn fixture(name: &str) -> (Vec<Ap1>, Repair) {
    let image = (0u32..160 * 160)
        .map(|i| {
            let x = (i % 160) as f32;
            let y = (i / 160) as f32;
            match name {
                "zero-seed" => {
                    [if ((31. ..73.).contains(&x) && (31. ..73.).contains(&y))
                        || (x >= 117. && y >= 117.)
                    {
                        0.
                    } else {
                        1e-17
                    }; 3]
                }
                "flat" => [0.18; 3],
                "periodic" => [
                    if i % 8 < 4 { 0.2 } else { 0.4 },
                    if i / 160 % 8 < 4 { 0.1 } else { 0.3 },
                    0.2,
                ],
                "hdr" => [
                    -1e25 + x * 2e23 + y * 1e23,
                    1e28 + x * 2e26 - y * 1e26,
                    -1e27 + x * y * 1e24,
                ],
                _ => [
                    0.1 + x * 0.002 + y * 0.001,
                    0.3 + x * 0.001 - y * 0.001,
                    0.05 + x * 0.0007 + y * 0.002,
                ],
            }
        })
        .collect();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        donor: DonorChoice::Automatic,
        support: SupportMask::new(
            [44, 44],
            [16, 16],
            (0..256)
                .map(|i| {
                    let x = (i % 16) as f64 - 7.5;
                    let y = (i / 16) as f64 - 7.5;
                    x * x + y * y <= 49.
                })
                .collect(),
        )
        .unwrap(),
    };
    (image, repair)
}
type PixelBits = Vec<([u32; 2], [u32; 3])>;
type DonorBits = Vec<([u32; 4], [u32; 2], u64, usize, u64)>;
fn bits(e: &EvaluatedRepair) -> (PixelBits, DonorBits) {
    (
        e.pixels().map(|(p, v)| (p, v.map(f32::to_bits))).collect(),
        e.donors
            .iter()
            .map(|d| {
                (
                    d.target,
                    d.source,
                    d.context_mse.to_bits(),
                    d.paired_context_pixels,
                    d.matching_score.to_bits(),
                )
            })
            .collect(),
    )
}
fn compare(screen: &mut dyn Screener, name: &str) {
    let (image, repair) = fixture(name);
    let active = SourceRegion {
        x: 0,
        y: 0,
        width: 160,
        height: 160,
    };
    let base = |[x, y]: [u32; 2]| Ok(image[(y * 160 + x) as usize]);
    let expected = evaluate(&repair, active, base).unwrap();
    let actual = evaluate_with(
        &repair,
        active,
        base,
        &mut Execution {
            screener: Some(screen),
            cancelled: &|| false,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(bits(&expected), bits(&actual), "{name}");
    if name == "zero-seed" {
        assert_eq!(actual.donors[0].matching_score, 0.);
    }
}

fn check_paired_ceilings(screen: &mut dyn Screener) {
    let image = (0..512)
        .map(|i| [i as f32 * 0.01, (i % 31) as f32 * 0.02, -(i as f32) * 0.005])
        .collect::<Vec<_>>();
    let gradients = (0..512).map(|i| [i as f32 * 0.001; 6]).collect::<Vec<_>>();
    let known = vec![true; 256];
    let valid = vec![true; 256];
    for paired_count in [127, 128, 129] {
        for weight in [1., 0.15, 0.5f64.next_down(), 0.5f64.next_up()] {
            let mut mask = vec![false; 512];
            let mut gradient_known = vec![true; 512];
            for i in paired_count..256 {
                let s = (i / 16) * 32 + 16 + i % 16;
                mask[s] = true;
                gradient_known[s] = false;
            }
            let weights = vec![weight; 256];
            let context = MatchContext {
                image: &image,
                mask: &mask,
                gradients: &gradients,
                gradient_known: &gradient_known,
                width: 32,
                target: [0, 0],
                source: [16, 0],
                size: [16, 16],
                known: &known,
                valid_target: &valid,
                weights: &weights,
                center: [0.; 3],
                contrast: 1.,
                gradient_scale: 1.,
            };
            let data = pack(context, &|| false).unwrap();
            let packed = pack_weights(&weights);
            let totals = MatchWeights::new(&weights, &valid);
            let exact_paired = (0..paired_count).fold(0., |sum, _| sum + weight);
            let result = screen
                .screen(
                    ScreenInput {
                        features: &data,
                        weights: &packed,
                        candidates: &[[16, 0]],
                        width: 32,
                        target: [0, 0],
                        local_width: 16,
                        factors: [0.; 2],
                        ceiling: f32::INFINITY,
                        paired_weights: true,
                        upper_totals: [0; 2],
                    },
                    &|| false,
                )
                .unwrap()
                .unwrap()[0];
            assert!(
                result[3] as f64 / 256. >= exact_paired && result[4] as f64 / 256. >= exact_paired
            );
            let reference = matching_score(context);
            if weight == 1. {
                assert_eq!(reference.is_ok(), paired_count >= 128);
            }
            let insufficient = result[3] as f64 / 256. < (totals.color * 0.5).max(1e-12)
                || result[4] as f64 / 256. < (totals.gradient * 0.5).max(1e-12);
            if insufficient {
                assert!(reference.is_err());
            }
            if let Ok(score) = reference {
                assert!(lower_score(result, totals) <= score);
            }
        }
    }
}

#[test]
fn paired_weight_ceilings_preserve_half_coverage_and_mode_boundaries() {
    let totals = MatchWeights {
        color: 1.,
        gradient: 1.,
    };
    assert!(!paired_coverage_needed([0.875, 0.875], totals));
    assert!(paired_coverage_needed([0.875f64.next_down(), 1.], totals));
    assert!(paired_coverage_needed([1., 0.875f64.next_down()], totals));
    assert!(!paired_coverage_needed([0.875f64.next_up(), 1.], totals));
    check_paired_ceilings(&mut ScalarScreen {
        calls: 0,
        prefix: usize::MAX,
    });
}

#[test]
fn conservative_prefixes_and_seeded_zero_preserve_exact_rank_and_repaired_bits() {
    for prefix in [1, 128, usize::MAX] {
        let mut screen = ScalarScreen { calls: 0, prefix };
        for name in ["gradient", "hdr", "periodic", "flat", "zero-seed"] {
            compare(&mut screen, name);
        }
        assert!(screen.calls > 0);
    }
}

#[test]
fn cancelled_preparation_is_none_without_sampling_or_screening() {
    let (_, repair) = fixture("gradient");
    let mut screen = ScalarScreen {
        calls: 0,
        prefix: usize::MAX,
    };
    let samples = Cell::new(0);
    let result = evaluate_with(
        &repair,
        SourceRegion {
            x: 0,
            y: 0,
            width: 160,
            height: 160,
        },
        |_| {
            samples.set(samples.get() + 1);
            Ok([0.; 3])
        },
        &mut Execution {
            screener: Some(&mut screen),
            cancelled: &|| true,
        },
    )
    .unwrap();
    assert!(result.is_none());
    assert_eq!(samples.get(), 0);
    assert_eq!(screen.calls, 0);
    let calls = Cell::new(0);
    assert!(
        distance_to_unknown(&vec![true; 4096], 64, 64, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 2
        })
        .is_none()
    );
}

#[test]
fn screening_errors_propagate_and_cancelled_screening_is_not_a_failure() {
    struct Failing;
    impl Screener for Failing {
        fn admit(&self, _: usize, _: usize, _: usize) -> bool {
            true
        }
        fn screen(
            &mut self,
            _: ScreenInput<'_>,
            _: &dyn Fn() -> bool,
        ) -> Result<Option<Vec<[u64; 5]>>, Error> {
            Err(Error::Gpu("screening failed".into()))
        }
    }
    struct Cancel<'a>(&'a Cell<bool>);
    impl Screener for Cancel<'_> {
        fn admit(&self, _: usize, _: usize, _: usize) -> bool {
            true
        }
        fn screen(
            &mut self,
            _: ScreenInput<'_>,
            _: &dyn Fn() -> bool,
        ) -> Result<Option<Vec<[u64; 5]>>, Error> {
            self.0.set(true);
            Ok(None)
        }
    }
    let (image, repair) = fixture("gradient");
    let active = SourceRegion {
        x: 0,
        y: 0,
        width: 160,
        height: 160,
    };
    let base = |[x, y]: [u32; 2]| Ok(image[(y * 160 + x) as usize]);
    let result = evaluate_with(
        &repair,
        active,
        base,
        &mut Execution {
            screener: Some(&mut Failing),
            cancelled: &|| false,
        },
    );
    assert!(matches!(result,Err(Error::Gpu(message)) if message=="screening failed"));
    let stop = Cell::new(false);
    let mut screen = Cancel(&stop);
    assert!(
        evaluate_with(
            &repair,
            active,
            base,
            &mut Execution {
                screener: Some(&mut screen),
                cancelled: &|| stop.get()
            }
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn quantized_terms_bound_reference_subtractions_and_normalizations() {
    let mut random = 0x7a2c_94e5_183d_620bu64;
    let mut next = || {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        random
    };
    let mut positive = 0;
    for i in 0..100_000 {
        let exponent = match i % 7 {
            0 => -149,
            1 => -100,
            2 => -20,
            3 => 0,
            4 => 30,
            5 => 100,
            _ => 125,
        };
        let a = ((next() as i64 as f64) / i64::MAX as f64 * 2f64.powi(exponent)) as f32 as f64;
        let b = ((next() as i64 as f64) / i64::MAX as f64 * 2f64.powi(exponent)) as f32 as f64;
        let center = if i % 2 == 0 { a } else { 0. };
        let scale = 2f64.powi(exponent).max(1e-18) * (0.1 + (next() % 1000) as f64 / 1000.);
        let weight = 0.15 + 0.85 * (next() % 10001) as f64 / 10000.;
        for gradient in [false, true] {
            let an = if gradient {
                a / scale
            } else {
                (a - center) / scale
            };
            let bn = if gradient {
                b / scale
            } else {
                (b - center) / scale
            };
            let aq = quantize(an);
            let bq = quantize(bn);
            let integer = if aq == i16::MIN || bq == i16::MIN {
                0
            } else {
                let gap = ((i32::from(aq) - i32::from(bq)).abs() - 3).clamp(0, 4095) as u64;
                gap * gap * (weight * 256.).floor() as u64
            };
            let lower = integer as f64 / (1u64 << 28) as f64 * (1. - 2f64.powi(-24));
            let delta = if gradient { (a - b) / scale } else { an - bn };
            assert!(lower <= delta.powi(2) * weight);
            positive += usize::from(lower > 0.);
        }
    }
    assert!(positive > 10_000);
    for q in [-32767i32, -4096, -1, 0, 1, 4096, 32767] {
        let edge = q as f64 / 1024.;
        for value in [edge.next_down(), edge, edge.next_up()] {
            let q = quantize(value);
            if q != i16::MIN {
                assert!(
                    value >= (f64::from(q) - 1.) / 1024. && value <= (f64::from(q) + 2.) / 1024.
                );
            }
        }
    }
    assert_eq!(quantize(f64::NAN), i16::MIN);
    assert_eq!(quantize(f64::INFINITY), i16::MIN);
}

#[cfg(feature = "editor")]
#[test]
fn hosted_gpu_screening_matches_cpu_and_scalar_integer_prefixes() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let gate = crate::gpu::SubmissionGate::default();
    let mut screen = crate::gpu::dust_screening::Screening::new(&device, &queue, &gate);
    for name in ["gradient", "hdr", "periodic", "flat", "zero-seed"] {
        compare(&mut screen, name);
    }
    let width = 96;
    let image = (0..width * width)
        .map(|i| {
            [
                i as f32 * 0.0001,
                (i % 31) as f32 * 0.01,
                -(i as f32) * 0.0002,
            ]
        })
        .collect::<Vec<_>>();
    let gradients = (0..image.len())
        .map(|i| [i as f32 * 0.00001; 6])
        .collect::<Vec<_>>();
    let mask = vec![false; image.len()];
    let gradient_known = vec![true; image.len()];
    let known = vec![true; 4096];
    let valid = vec![true; 4096];
    let weights = vec![1.; 4096];
    let context = MatchContext {
        image: &image,
        mask: &mask,
        gradients: &gradients,
        gradient_known: &gradient_known,
        width,
        target: [0, 0],
        source: [0, 0],
        size: [64, 64],
        known: &known,
        valid_target: &valid,
        weights: &weights,
        center: [0.; 3],
        contrast: 1.,
        gradient_scale: 1.,
    };
    let features = pack(context, &|| false).unwrap();
    let weights = vec![256 | (256 << 9); 4096];
    let positions = (0..=32)
        .flat_map(|y| (0..=32).map(move |x| [x, y]))
        .collect::<Vec<_>>();
    for ceiling in [f32::NEG_INFINITY, 0., f32::INFINITY, f32::NAN] {
        let input = ScreenInput {
            features: &features,
            weights: &weights,
            candidates: &positions,
            width: width as u32,
            target: [0, 0],
            local_width: 64,
            factors: [1e-12, 1e-12],
            ceiling,
            paired_weights: true,
            upper_totals: [0; 2],
        };
        let result = screen.screen(input, &|| false).unwrap().unwrap();
        for (source, sum) in positions.iter().zip(result) {
            assert!(sum[2] == 2048 || sum[2] == 4096);
            assert_eq!(sum, integer_sums(&input, *source, sum[2] as usize));
        }
    }
    check_paired_ceilings(&mut screen);
    assert!(!screen.admit(4_000_000, 4_000_000, 66049));
    let input = ScreenInput {
        features: &features,
        weights: &weights,
        candidates: &positions,
        width: width as u32,
        target: [0, 0],
        local_width: 64,
        factors: [0.; 2],
        ceiling: 0.,
        paired_weights: true,
        upper_totals: [0; 2],
    };
    let polls = Cell::new(0);
    assert!(
        screen
            .screen(input, &|| {
                polls.set(polls.get() + 1);
                polls.get() > 4
            })
            .unwrap()
            .is_none()
    );
}

#[cfg(feature = "editor")]
#[test]
#[ignore = "private read-only AP1 corpus; set ANTITONE_DUST_CORPUS"]
fn retained_gpu_screening_preserves_all_donor_and_pixel_bits() {
    let manifest = std::env::var("ANTITONE_DUST_CORPUS").expect("set ANTITONE_DUST_CORPUS");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let gate = crate::gpu::SubmissionGate::default();
    let mut screen = crate::gpu::dust_screening::Screening::new(&device, &queue, &gate);
    let mut comparisons = 0;
    for line in std::fs::read_to_string(manifest).unwrap().lines() {
        let f = line.split('\t').collect::<Vec<_>>();
        assert_eq!(f.len(), 7);
        let size = [f[3].parse::<u32>().unwrap(), f[4].parse::<u32>().unwrap()];
        let bytes = std::fs::read(f[1]).unwrap();
        assert_eq!(bytes.len(), size[0] as usize * size[1] as usize * 12);
        let image = bytes
            .as_chunks::<12>()
            .0
            .iter()
            .map(|p| {
                std::array::from_fn::<_, 3, _>(|c| {
                    f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap())
                })
            })
            .collect::<Vec<_>>();
        let mask = std::fs::read(f[2]).unwrap();
        assert_eq!(mask.len(), image.len());
        let points = mask
            .iter()
            .enumerate()
            .filter_map(|(i, b)| (*b != 0).then_some([i as u32 % size[0], i as u32 / size[0]]))
            .collect::<Vec<_>>();
        let origin = [
            points.iter().map(|p| p[0]).min().unwrap(),
            points.iter().map(|p| p[1]).min().unwrap(),
        ];
        let end = [
            points.iter().map(|p| p[0]).max().unwrap() + 1,
            points.iter().map(|p| p[1]).max().unwrap() + 1,
        ];
        let ss = [end[0] - origin[0], end[1] - origin[1]];
        let support = SupportMask::new(
            origin,
            ss,
            (0..ss[0] * ss[1])
                .map(|i| {
                    mask[((origin[1] + i / ss[0]) * size[0] + origin[0] + i % ss[0]) as usize] != 0
                })
                .collect(),
        )
        .unwrap();
        let mut choices = vec![DonorChoice::Automatic];
        let dx = f[5].parse::<i32>().unwrap();
        let dy = f[6].parse::<i32>().unwrap();
        if dx != i32::MAX {
            choices.push(DonorChoice::Offset([dx, dy]));
        }
        for donor in choices {
            let repair = Repair {
                id: RepairId(1),
                enabled: true,
                origin: RepairOrigin::Manual,
                support: support.clone(),
                donor,
            };
            let active = SourceRegion {
                x: 0,
                y: 0,
                width: size[0],
                height: size[1],
            };
            let base = |[x, y]: [u32; 2]| Ok(image[(y * size[0] + x) as usize]);
            let clock = std::time::Instant::now();
            let expected = evaluate(&repair, active, base).unwrap();
            let cpu = clock.elapsed();
            let clock = std::time::Instant::now();
            let actual = evaluate_with(
                &repair,
                active,
                base,
                &mut Execution {
                    screener: Some(&mut screen),
                    cancelled: &|| false,
                },
            )
            .unwrap()
            .unwrap();
            let gpu = clock.elapsed();
            assert_eq!(bits(&expected), bits(&actual), "{} {donor:?}", f[0]);
            eprintln!("{} {donor:?} cpu={cpu:?} gpu={gpu:?} exact", f[0]);
            comparisons += 1;
        }
    }
    assert!(comparisons > 0);
    eprintln!("exact retained comparisons={comparisons}");
}
