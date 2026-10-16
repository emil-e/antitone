// Full-raster diagnostics, outside all timed editing submissions.
@group(0) @binding(0)
var<storage, read> reference: array<vec4f>;
@group(0) @binding(1)
var<storage, read> candidate: array<vec4f>;
@group(0) @binding(2)
var reference_target: texture_2d<f32>;
@group(0) @binding(3)
var candidate_target: texture_2d<f32>;
// Lanes: normalized AP1 error, absolute AP1 error, flag mismatches, nonfinite channels,
// target-linear error, target-linear failures, encoded error, AP1 failures.
// Error lanes hold maxima as f32 bits; the other lanes count discrepancies.
@group(0) @binding(4)
var<storage, read_write> result: array<atomic<u32>, 8>;
var<workgroup> partial: array<atomic<u32>, 8>;

fn finite(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7f800000u) != 0x7f800000u;
}

fn encoded(x: f32) -> f32 {
    if x <= 0.0031308 {
        return 12.92 * x;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

@compute @workgroup_size(64)
fn main(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(local_invocation_index) lane: u32,
    @builtin(num_workgroups) groups: vec3u,
) {
    if lane < 8u {
        atomicStore(&partial[lane], 0u);
    }
    workgroupBarrier();
    let index = id.x + id.y * groups.x * 64u;
    if index < arrayLength(&reference) {
        let size = textureDimensions(reference_target);
        let xy = vec2i(i32(index % size.x), i32(index / size.x));
        let r = reference[index];
        let a = candidate[index];
        let rt = textureLoad(reference_target, xy, 0);
        let at = textureLoad(candidate_target, xy, 0);
        if bitcast<u32>(r.a) != bitcast<u32>(a.a) {
            atomicAdd(&partial[2], 1u);
        }
        for(var c = 0u; c < 3u; c++) {
            if !finite(a[c]) || !finite(r[c]) || !finite(at[c]) || !finite(rt[c]) {
                atomicAdd(&partial[3], 1u);
            } else {
                let delta = abs(a[c] - r[c]);
                let units = delta / (2e-5 + 2e-4 * abs(r[c]));
                atomicMax(&partial[0], bitcast<u32>(units));
                atomicMax(&partial[1], bitcast<u32>(delta));
                if units > 1.0 {
                    atomicAdd(&partial[7], 1u);
                }
                let render_delta = abs(at[c] - rt[c]);
                atomicMax(&partial[4], bitcast<u32>(render_delta));
                if render_delta > 2e-5 {
                    atomicAdd(&partial[5], 1u);
                }
                atomicMax(&partial[6], bitcast<u32>(abs(encoded(at[c]) - encoded(rt[c]))));
            }
        }
    }
    workgroupBarrier();
    if lane < 8u {
        let value = atomicLoad(&partial[lane]);
        if lane == 2u || lane == 3u || lane == 5u || lane == 7u {
            atomicAdd(&result[lane], value);
        } else {
            atomicMax(&result[lane], value);
        }
    }
}
