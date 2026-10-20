// Original-sample reference/picker processing retains compensated colour arithmetic.
fn photographic_grade(input: array<Triple, 3>) -> array<Triple, 3> {
    return apply_creative_grade(input, global_creative_grade());
}

fn local_grade(input: array<Triple, 3>, index: u32) -> array<Triple, 3> {
    if local_context.count.x == 0u {
        return input;
    }
    let source = source_pixel(index);
    // Padded refinement samples are not original source pixels.
    if any(source < local_context.area.xy)
        || any(source >= local_context.area.xy + local_context.area.zw) {
        return input;
    }
    var rgb = input;
    for(var stage = 0u; stage < local_context.count.x; stage++) {
        var coverage = local_coverage(stage, source);
        if local_stages[stage].flags.y != 0u {
            coverage = precise_sub(number(1.0), coverage);
        }
        let weight = precise_mul(coverage, local_constant(stage, 1u, 3u));
        if precise_eq(weight, number(0.0)) {
            continue;
        }
        let ev = local_constant(stage, 2u, 0u);
        if !precise_eq(ev, number(0.0)) {
            let gain = precise_exp(precise_mul(weight, ev));
            for(var c = 0u; c < 3u; c++) {
                rgb[c] = precise_mul(rgb[c], gain);
            }
        }

        if local_stages[stage].flags.z != 0u {
            var cones = local_matrix(false, rgb);
            for(var c = 0u; c < 3u; c++) {
                cones[c] = precise_mul(
                        cones[c],
                        precise_exp(precise_mul(weight, local_constant(stage, 2u, c + 1u))),
                    );
            }
            rgb = local_matrix(true, cones);
        }

        let amount = precise_add(
                number(1.0),
                precise_mul(weight, precise_sub(local_constant(stage, 3u, 0u), number(1.0))),
            );
        let saturation = precise_add(
                number(1.0),
                precise_mul(weight, precise_sub(local_constant(stage, 3u, 3u), number(1.0))),
            );
        if precise_eq(amount, number(1.0)) && precise_eq(saturation, number(1.0)) {
            continue;
        }
        let colour = local_constant(stage, 3u, 2u);
        var qp = number(0.0);
        var q0 = number(0.0);
        var black = number(0.0);
        if !precise_eq(amount, number(1.0)) {
            qp = local_constant(stage, 3u, 1u);
            if !precise_eq(colour, number(0.0)) {
                q0 = precise_add(qp, precise_mul(amount, precise_sub(CCT_OFFSET, qp)));
                black = cct_decode(q0);
            }
        }
        rgb = apply_creative_grade(
                rgb,
                CreativeGrade(
                    amount,
                    qp,
                    q0,
                    black,
                    colour,
                    saturation,
                    local_constant(stage, 4u, 0u),
                ),
            );
    }
    return rgb;
}
