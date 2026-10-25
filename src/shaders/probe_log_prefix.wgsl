// Test-only pre-exposure cut. Reuse the 40-byte record for anchored log triples;
// flags carry the original floor bits, two sign bits/channel and exact-zero bits.
fn probe_anchored_response(scan: ConvertedScan) -> ProcessedPixel {
    var q: array<Triple, 3>;
    var flags = 0u;
    let floor = Triple(parameters.dmin_floor.w, parameters.dmin_low.w, parameters.dmin_tail.w);
    for(var c = 0u; c < 3u; c++) {
        var s = Triple(scan.high[c], scan.low[c], scan.tail[c]);
        if !precise_le(floor, s) {
            flags |= 1u << c;
            s = floor;
        }
        let base = Triple(
                parameters.dmin_floor[c],
                parameters.dmin_low[c],
                parameters.dmin_tail[c],
            );
        let l = precise_log_ratio(base, s);
        let beta = Triple(parameters.shape.x, parameters.shape.z, parameters.shape_tail.x);
        let k = Triple(parameters.shape.y, parameters.shape.w, parameters.shape_tail.y);
        let gamma = Triple(
                parameters.gamma_mode[c],
                parameters.gamma_low[c],
                parameters.gamma_tail[c],
            );
        let density = precise_div(l, LN10_PRECISE);
        let a = precise_div(precise_mul(beta, l), precise_sub(gamma, precise_mul(k, density)));
        if precise_eq(a, number(0.0)) {
            q[c] = number(0.0);
            flags |= 1u << (9u + c);
        } else {
            let anchor = Triple(
                    parameters.anchor[c],
                    parameters.anchor_low[c],
                    parameters.anchor_tail[c],
                );
            q[c] = precise_div(precise_sub(precise_log_abs_expm1(a), anchor), beta);
            // Preserve sign(a.h), including signed zero, without another scalar word.
            let bits = bitcast<u32>(sign(a.h));
            let code = (bits >> 31u) | select(0u, 2u, (bits & 0x7fffffffu) != 0u);
            flags |= code << (3u + 2u * c);
        }
    }
    return ProcessedPixel(q, flags);
}

fn probe_expose_response(prefix: ProcessedPixel) -> ProcessedPixel {
    var q: array<Triple, 3>;
    let gain = Triple(parameters.anchor.w, parameters.anchor_low.w, parameters.anchor_tail.w);
    for(var c = 0u; c < 3u; c++) {
        if (prefix.flags & (1u << (9u + c))) != 0u {
            q[c] = number(0.0);
        } else {
            let code = (prefix.flags >> (3u + 2u * c)) & 3u;
            let bits = ((code & 1u) << 31u) | select(0u, 0x3f800000u, (code & 2u) != 0u);
            q[c] = precise_mul(
                    number(bitcast<f32>(bits)),
                    precise_exp(precise_add(prefix.rgb[c], gain)),
                );
        }
    }
    return ProcessedPixel(q, prefix.flags & 7u);
}

@compute @workgroup_size(WORKGROUP_SIZE)
fn build_log_prefix(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = probe_index(id, groups);
    if index >= source_pixel_count() {
        return;
    }
    prefix_pixels[index] = probe_anchored_response(convert_source(scans[index], index));
}

@compute @workgroup_size(WORKGROUP_SIZE)
fn log_prefix_main(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = probe_index(id, groups);
    if index >= source_pixel_count() {
        return;
    }
    if parameters.stage == STAGE_SOURCE {
        positives[index] = process_camera_source(convert_source(scans[index], index));
        return;
    }
    let native = probe_expose_response(prefix_pixels[index]);
    let global = photographic_post(camera_positive_to_ap1(native.rgb));
    positives[index] = finish_pixel(ProcessedPixel(local_grade(global, index), native.flags));
}
