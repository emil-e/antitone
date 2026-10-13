use super::*;

// Provisional baselines for judging endpoints, not recovered or accepted grades.
const EXPOSURES: [f64; 3] = [2.5, 3., 2.];
const SHORTLIST: [&str; 13] = [
    "identity",
    "contrast-down-0",
    "contrast-down-2",
    "contrast-down-4",
    "contrast-up-0",
    "contrast-up-2",
    "contrast-up-4",
    "sat-maximum-down-0",
    "sat-maximum-down-2",
    "sat-maximum-down-4",
    "sat-maximum-up-0",
    "sat-maximum-up-2",
    "sat-maximum-up-4",
];

fn comparison_variants() -> Result<Vec<Variant>> {
    Ok(variants()?
        .into_iter()
        .filter(|variant| SHORTLIST.contains(&variant.name.as_str()))
        .collect())
}

fn render(
    directory: &Path,
    renderer: &CpuRenderer,
    fixture: &Fixture,
    variant: &Variant,
    exposure_gain: f64,
    name: &str,
) -> Result<()> {
    // Exposure brackets are linear AP1 gains BEFORE creative processing/ACES,
    // never brightness changes to rendered images or model-specific fitting.
    let working = fixture
        .inputs
        .iter()
        .map(|rgb| variant.apply(exposure_gain * *rgb))
        .collect::<operators::Result<Vec<_>>>()?;
    let rendered = renderer.render(&working)?;
    let pixels: Vec<_> = rendered
        .chunks_exact(fixture.samples)
        .map(|p| (p.iter().map(|v| v.as_dvec3()).sum::<DVec3>() / fixture.samples as f64).as_vec3())
        .collect();
    write_image(
        &directory.join(format!("{name}.tif")),
        OutputTarget::DisplayP3,
        &pixels,
    )
}

pub fn run(root: &Path, directory: &Path) -> Result<()> {
    let fixtures = photos(root, directory, EXPOSURES)?;
    let variants = comparison_variants()?;
    let renderer = CpuRenderer::new(OutputTarget::DisplayP3)?;
    for (fixture, exposure) in fixtures.iter().zip(EXPOSURES) {
        for variant in &variants {
            render(
                directory,
                &renderer,
                fixture,
                variant,
                1.,
                &format!("{}-display-p3-{}", fixture.name, variant.name),
            )?;
        }
        let bracket: &[f64] = match fixture.name {
            "ring" => &[2.5, 3., 3.5],
            "canopy" => &[1.5, 2., 2.5],
            _ => &[],
        };
        for ev in bracket {
            render(
                directory,
                &renderer,
                fixture,
                &Variant::identity(),
                (ev - exposure).exp2(),
                &format!("{}-exposure-{ev:.1}", fixture.name),
            )?;
        }
        println!(
            "rendered {}: baseline +{exposure} EV, P3 endpoints/midpoints and exposure bracket",
            fixture.name
        );
    }
    fs::write(
        directory.join("index.html"),
        include_str!("simple-exposure.html"),
    )?;
    fs::write(
        directory.join("comparison.html"),
        include_str!("simple-comparison.html"),
    )?;
    fs::write(directory.join("simple.css"), include_str!("simple.css"))?;
    fs::write(
        directory.join("run.txt"),
        format!(
            "OCIO={}\nmode=simple\nfixtures=3\nvariants={}\nexposure-bracket-images=6\nsize={WIDTH}x{HEIGHT}\npivot={PIVOT}\nbaseline-exposures dog/ring/canopy={EXPOSURES:?}\nExposures are provisional. All comparisons share the same per-photo exposure/WB.\nHalfway is 50/50 endpoint-output interpolation in linear AP1 before ACES, not parameter interpolation.\nPrint-density power remains in the full research matrix only.\nSource recipe: photographic-inputs.txt; equations/reproduction: docs/research/tools/contrast-saturation/README.md\n",
            ocio_version(),
            variants.len()
        ),
    )?;
    println!("exposure sheet={}", directory.join("index.html").display());
    println!(
        "endpoint comparison={}",
        directory.join("comparison.html").display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_shortlist_contains_only_uncombined_endpoints_and_midpoints() {
        let variants = comparison_variants().unwrap();
        assert_eq!(variants.len(), SHORTLIST.len());
        for name in SHORTLIST {
            assert_eq!(variants.iter().filter(|v| v.name == name).count(), 1);
        }
        for variant in variants {
            assert!(variant.contrast == 1. || variant.saturation == 1.);
            assert!([0., 0.5, 1.].contains(&variant.contrast_blend));
            assert!([0., 0.5, 1.].contains(&variant.saturation_blend));
            assert!(matches!(variant.density, Saturation::Maximum));
            assert_eq!(variant.density_amount, variant.saturation);
        }
    }

    #[test]
    fn simple_midpoints_average_endpoint_outputs_before_rendering() {
        let variants = comparison_variants().unwrap();
        for (mid, left, right) in [
            ("contrast-down-2", "contrast-down-0", "contrast-down-4"),
            ("contrast-up-2", "contrast-up-0", "contrast-up-4"),
            (
                "sat-maximum-down-2",
                "sat-maximum-down-0",
                "sat-maximum-down-4",
            ),
            ("sat-maximum-up-2", "sat-maximum-up-0", "sat-maximum-up-4"),
        ] {
            let get = |name| variants.iter().find(|v| v.name == name).unwrap();
            for rgb in [
                DVec3::ZERO,
                DVec3::splat(PIVOT),
                MATCH,
                DVec3::new(1., -0.4, 0.02),
                DVec3::new(4., 0.3, 0.02),
            ] {
                let expected =
                    0.5 * get(left).apply(rgb).unwrap() + 0.5 * get(right).apply(rgb).unwrap();
                let actual = get(mid).apply(rgb).unwrap();
                assert!(
                    (actual - expected).abs().max_element()
                        <= 2e-12 * (1. + expected.abs().max_element())
                );
                let identity = Variant {
                    contrast: 1.,
                    saturation: 1.,
                    density_amount: 1.,
                    ..get(mid).clone()
                };
                assert_eq!(identity.apply(rgb).unwrap(), rgb);
            }
        }
    }
}
