use crate::Error;

const HUE_ARRAY: &str = "aces_gamut_cusp_table_0_hues_array";
const HUE_COUNT: usize = 363;

/// Adapt resource declarations only; preserve the pinned OCIO function bodies and values.
pub(crate) fn adapt(pinned: &str) -> Result<(String, Vec<f32>), Error> {
    let mut generated = pinned.to_owned();
    for (binding, name) in [(1, "aces_reach_m_table_0"), (2, "aces_gamut_cusp_table_0")] {
        let declaration =
            format!("layout(set=0, binding = {binding}) uniform sampler2D {name}Sampler;");
        if generated.matches(&declaration).count() != 1 {
            return Err(Error::Gpu(
                "unexpected pinned OCIO resource declaration".into(),
            ));
        }
        generated = generated.replace(
            &declaration,
            &format!(
                "layout(set=0, binding = {binding}) uniform texture2D {name};\n#define {name}Sampler sampler2D({name}, aces_sampler)"
            ),
        );
    }

    // A dynamically indexed constant array is very expensive on the
    // tested Metal path. Store exactly its f32 literal values in immutable shared
    // resources instead; no lookup interpolation or ACES arithmetic changes.
    let prefix = format!("const float {HUE_ARRAY}");
    if pinned.matches(&prefix).count() != 1 {
        return Err(Error::Gpu("unexpected pinned OCIO hue declaration".into()));
    }
    let declaration = pinned
        .lines()
        .find(|line| line.starts_with(&prefix))
        .ok_or_else(|| Error::Gpu("unexpected pinned OCIO hue declaration".into()))?;
    let expected = format!("{prefix}[{HUE_COUNT}] = float[{HUE_COUNT}](");
    let literals = declaration
        .strip_prefix(&expected)
        .and_then(|values| values.strip_suffix(");"))
        .ok_or_else(|| Error::Gpu("unexpected pinned OCIO hue declaration".into()))?;
    let hues: Vec<f32> = literals
        .split(',')
        .map(|literal| {
            literal
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| Error::Gpu("unexpected pinned OCIO hue values".into()))
        })
        .collect::<Result<_, _>>()?;
    if hues.len() != HUE_COUNT {
        return Err(Error::Gpu("unexpected pinned OCIO hue count".into()));
    }
    generated = generated.replace(
        declaration,
        &format!("layout(set=0,binding=5,std430) readonly buffer Hues {{ float {HUE_ARRAY}[]; }};"),
    );
    Ok((generated, hues))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PINNED: [&str; 2] = [
        include_str!("../../assets/ocio/srgb-ap1.glsl"),
        include_str!("../../assets/ocio/display-p3-ap1.glsl"),
    ];

    #[test]
    fn adaptation_preserves_both_targets_functions_and_exact_literal_values() {
        for pinned in PINNED {
            let (adapted, hues) = adapt(pinned).unwrap();
            assert_eq!(hues.len(), HUE_COUNT);
            let declaration = pinned
                .lines()
                .find(|line| line.starts_with("const float aces_gamut_cusp_table_0_hues_array"))
                .unwrap();
            let literals = declaration.split_once("float[363](").unwrap().1;
            for (actual, literal) in hues
                .iter()
                .zip(literals.strip_suffix(");").unwrap().split(','))
            {
                // GLSL's decimal float literals and the buffer are both f32 values.
                assert_eq!(
                    actual.to_bits(),
                    literal.trim().parse::<f32>().unwrap().to_bits()
                );
            }
            let functions = |code: &str| {
                code.lines()
                    .filter(|line| {
                        !line.starts_with("layout(set=0,")
                            && !line.starts_with("#define aces_")
                            && !line.starts_with("const float aces_gamut_cusp_table_0_hues_array")
                    })
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            assert_eq!(functions(pinned), functions(&adapted));
            assert!(adapted.contains("binding=5,std430) readonly buffer Hues"));
            assert!(!adapted.contains(declaration));
        }
    }

    #[test]
    fn unexpected_resources_and_hue_payloads_reject_without_adaptation() {
        let pinned = PINNED[0];
        let declaration = pinned
            .lines()
            .find(|line| line.starts_with("const float aces_gamut_cusp_table_0_hues_array"))
            .unwrap();
        let prefix = "const float aces_gamut_cusp_table_0_hues_array[363] = float[363](";
        let values = declaration
            .strip_prefix(prefix)
            .unwrap()
            .strip_suffix(");")
            .unwrap();
        let cases = [
            pinned.replace(declaration, ""),
            pinned.replace(declaration, &format!("{declaration}\n{declaration}")),
            pinned.replace(declaration, &declaration.replace("[363]", "[362]")),
            pinned.replace(
                declaration,
                &format!(
                    "{prefix}{});",
                    values.split(',').skip(1).collect::<Vec<_>>().join(",")
                ),
            ),
            pinned.replace(
                declaration,
                &format!("{prefix}NaN,{});", values.split_once(',').unwrap().1),
            ),
            pinned.replace(
                declaration,
                &format!("{prefix}1e99,{});", values.split_once(',').unwrap().1),
            ),
            pinned.replace(
                declaration,
                &format!(
                    "{prefix}not_a_literal,{});",
                    values.split_once(',').unwrap().1
                ),
            ),
            pinned.replace("binding = 1", "binding = 8"),
            format!(
                "{pinned}\nlayout(set=0, binding = 2) uniform sampler2D aces_gamut_cusp_table_0Sampler;"
            ),
        ];
        for candidate in cases {
            assert!(adapt(&candidate).is_err());
        }
        assert!(adapt(pinned).is_ok());
    }
}
