use super::*;
use crate::photographic::stages::look;

fn diagonal() -> (Reference, data::Pair) {
    static DEFINITION: OnceLock<Definition> = OnceLock::new();
    let definition = DEFINITION.get_or_init(|| Definition {
        schema: 1,
        negative: "analytic".into(),
        exposure_rows: DMat3::IDENTITY.to_cols_array_2d(),
        dye_rows: DMat3::IDENTITY.to_cols_array_2d(),
        density_gain: 1.,
        base_density_offset: 1.25,
    });
    let reference = Reference {
        definition,
        exposure: DMat3::IDENTITY,
        dye_basis: DMat3::IDENTITY,
        lamp_scale: 10_f64.powf(definition.base_density_offset),
        samples: [DVec3::X, DVec3::Y, DVec3::Z]
            .into_iter()
            .enumerate()
            .map(|(index, dyes)| Sample {
                index,
                base: [0.2, 0.3, 0.4][index] + 1.25,
                dyes,
            })
            .collect(),
    };
    let pair = data::Pair {
        negative: "analytic".into(),
        paper: "analytic".into(),
        weights: vec![DVec3::X * 2., DVec3::Y * 3., DVec3::Z * 4.],
        preflash: DVec3::new(0.1, 0.2, 0.3),
        print_exposure: 2.5,
    };
    (reference, pair)
}

#[test]
fn separate_dye_bands_match_analytic_exposure_including_base_and_preflash() {
    let (r, pair) = diagonal();
    let curve = crate::looks::characteristic::reference();
    for rgb in [
        DVec3::ZERO,
        DVec3::new(0.003, 0.02, 0.18),
        DVec3::new(4., 0.008, 0.03),
    ] {
        let actual = r.expose(rgb, &pair);
        for c in 0..3 {
            let raw = [2., 3., 4.][c] * 10_f64.powf(-[0.2, 0.3, 0.4][c] - curve.density(rgb[c]))
                + pair.preflash[c];
            let expected = ((raw + 1e-10) * 2.5 + 1e-10).log10();
            assert!((actual[c] - expected).abs() < 1e-13);
        }
    }
}

#[test]
fn signed_dye_coordinates_and_spectra_are_retained_in_total_density() {
    let (mut r, pair) = diagonal();
    r.dye_basis = matrix([[2., -1., 0.], [0., 1., 0.], [0., 0., 1.]]);
    r.samples[1].dyes.x = -0.1;
    let rgb = DVec3::new(0.003, 0.18, 0.02);
    let a = r.amounts(rgb);
    assert!(a.x < 0.);
    let mut raw = pair.preflash;
    for s in &r.samples {
        let density = s.base + s.dyes.dot(a);
        assert!(density > 0.);
        raw += pair.weights[s.index] * 10_f64.powf(1.25 - density);
    }
    let expected = ((raw + DVec3::splat(1e-10)) * 2.5 + DVec3::splat(1e-10)).map(f64::log10);
    assert!((r.expose(rgb, &pair) - expected).abs().max_element() < 1e-13);
}

#[test]
fn pre_and_post_matrices_have_distinct_roles_around_the_shared_curve() {
    let (mut r, _) = diagonal();
    r.exposure = matrix([[0.5, 0.5, 0.], [0., 1., 0.], [0., 0., 1.]]);
    r.dye_basis = matrix([[2., -1., 0.], [0., 1., 0.], [0., 0., 1.]]);
    let rgb = DVec3::new(0.03, 0.003, 0.18);
    let curve = crate::looks::characteristic::reference();
    let expected = 2. * curve.density(0.0165) - curve.density(0.003);
    assert!((r.amounts(rgb).x - expected).abs() < 1e-14);
    let collapsed = (r.dye_basis * r.exposure * rgb).map(|v| curve.density(v));
    assert!((r.amounts(rgb).x - collapsed.x).abs() > 0.01);
}

#[test]
fn catalogue_neutrals_share_one_curve_and_density_gain_across_papers() {
    let r = reference().unwrap();
    let curve = crate::looks::characteristic::reference();
    for ev in -12..=12 {
        let x = curve.gray * 2_f64.powi(ev);
        let expected = DVec3::splat(curve.density(x) * r.definition.density_gain);
        assert!((r.amounts(DVec3::splat(x)) - expected).abs().max_element() < 1e-12);
    }
}

#[test]
fn calibrated_filter_is_passive_over_the_complete_authored_input_cube() {
    let r = reference().unwrap();
    let curve = crate::looks::characteristic::reference();
    let qmin = crate::looks::authoring::DOMAIN_MIN;
    let qmax = crate::looks::authoring::domain_max();
    let mut boxes = vec![(DVec3::splat(qmin), DVec3::splat(qmax))];
    let rows = r.exposure.transpose().to_cols_array_2d();
    let mut visited = 0;
    while let Some((lo, hi)) = boxes.pop() {
        visited += 1;
        assert!(
            visited < 100_000,
            "passive-domain interval verification did not finish"
        );
        let low = lo.map(look::decode);
        let high = hi.map(look::decode);
        let emin = DVec3::from_array(std::array::from_fn(|c| {
            (0..3)
                .map(|k| rows[c][k] * if rows[c][k] >= 0. { low[k] } else { high[k] })
                .sum()
        }));
        let emax = DVec3::from_array(std::array::from_fn(|c| {
            (0..3)
                .map(|k| rows[c][k] * if rows[c][k] >= 0. { high[k] } else { low[k] })
                .sum()
        }));
        let fmin = emin.map(|x| curve.density(x));
        let fmax = emax.map(|x| curve.density(x));
        let bound = r
            .samples
            .iter()
            .map(|s| {
                let q = r.dye_basis.transpose() * s.dyes * r.definition.density_gain;
                s.base
                    + (0..3)
                        .map(|i| q[i] * if q[i] >= 0. { fmin[i] } else { fmax[i] })
                        .sum::<f64>()
            })
            .fold(f64::INFINITY, f64::min);
        if bound >= 1e-8 {
            continue;
        }
        let midpoint = ((lo + hi) * 0.5).map(look::decode);
        let a = r.amounts(midpoint);
        for s in &r.samples {
            assert!(s.base + s.dyes.dot(a) >= bound - 1e-10);
            assert!(s.base + s.dyes.dot(a) >= 0.);
        }
        let span = hi - lo;
        let axis = (0..3).max_by(|&i, &j| span[i].total_cmp(&span[j])).unwrap();
        let mid = (lo[axis] + hi[axis]) * 0.5;
        let mut left = hi;
        left[axis] = mid;
        let mut right = lo;
        right[axis] = mid;
        boxes.push((lo, left));
        boxes.push((right, hi));
    }
    assert!(visited > 1);
}

#[test]
fn malformed_definitions_reject_without_prohibiting_signed_effective_matrices() {
    let make = || serde_json::from_slice::<Definition>(DEFINITION).unwrap();
    make().validate().unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(DEFINITION).unwrap();
    value["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Definition>(value).is_err());
    let mut d = make();
    d.exposure_rows[0][0] += 0.1;
    assert!(d.validate().is_err());
    let mut d = make();
    d.dye_rows = [[1., 0., 0.]; 3];
    assert!(d.validate().is_err());
    let mut d = make();
    d.density_gain = f64::NAN;
    assert!(d.validate().is_err());
    let mut d = make();
    d.base_density_offset = -1.;
    assert!(d.validate().is_err());
}
