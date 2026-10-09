struct Parameters {
    dmin_floor: vec4f,
    gamma_mode: vec4f,
    anchor: vec4f,
    post: array<vec4f, 3>,
    dmin_low: vec4f,
    dmin_tail: vec4f,
    gamma_low: vec4f,
    shape: vec4f,
    anchor_low: vec4f,
    post_low: array<vec4f, 3>,
}
@group(0) @binding(0) var<storage, read> scans: array<vec2u>;
@group(0) @binding(1) var<storage, read_write> positives: array<vec4f>;
@group(0) @binding(2) var<uniform> parameters: Parameters;

fn expm1_stable(x: f32) -> f32 {
    if abs(x) < 0.001 {
        return x * (1.0 + x * (0.5 + x * (1.0 / 6.0 + x / 24.0)));
    }
    return exp(x) - 1.0;
}

fn log_ratio(a: f32, b: f32) -> f32 {
    let z = (a - b) / b;
    if abs(z) < 0.001 {
        return z * (1.0 + z * (-0.5 + z * (1.0 / 3.0 - z * 0.25)));
    }
    return log(a) - log(b);
}

fn invert_original(scan: f32, dmin: f32, gamma: f32, anchor: f32, log_gain: f32) -> f32 {
    let a = log_ratio(dmin, scan) / gamma;
    if a > 50.0 || anchor > 50.0 {
        if a >= 0.0 {
            // Fold common exposure into the ratio to avoid overflow before attenuation.
            return exp(a - anchor + log_gain) * (-expm1_stable(-a)) / (-expm1_stable(-anchor));
        }
        return expm1_stable(a) * exp(-anchor + log_gain) / (-expm1_stable(-anchor));
    }
    return expm1_stable(a) / expm1_stable(anchor) * exp(log_gain);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&scans) { return; }
    let scan = convert_camera(scans[index]);
    if parameters.gamma_mode.w > 1.5 {
        positives[index] = process_film(scan);
        return;
    }
    var value = scan.high;
    var mask = 0u;
    if parameters.gamma_mode.w > 0.5 {
        for (var channel = 0u; channel < 3u; channel++) {
            if value[channel] < parameters.dmin_floor.w {
                mask |= 1u << channel;
            }
            value[channel] = invert_original(
                max(value[channel], parameters.dmin_floor.w),
                parameters.dmin_floor[channel], parameters.gamma_mode[channel], parameters.anchor[channel], parameters.anchor.w);
        }
    }
    let post = mat3x3f(parameters.post[0].xyz, parameters.post[1].xyz, parameters.post[2].xyz);
    // The fourth component carries diagnostic bits, not display alpha.
    positives[index] = vec4f(post * value, f32(mask));
}
