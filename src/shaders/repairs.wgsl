struct RepairPixel {
    position: vec4u,
    value: vec4f,
}
@group(0) @binding(12)
var<storage, read> repair_pixels: array<RepairPixel>;
@group(0) @binding(13)
var<uniform> repair_tile: vec4u;

struct RepairView {
    bounds_output: vec4u,
    support: vec4u,
    window: vec4u,
    copy_region: vec4u,
    start: vec4f,
    dx: vec4f,
    dy: vec4f,
}
@group(0) @binding(14)
var<uniform> repair_view: RepairView;
@group(0) @binding(15)
var<storage, read> reconstructed: array<vec4f>;

@compute @workgroup_size(64)
fn repair_original(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&repair_pixels) {
        return;
    }
    let pixel = repair_pixels[id.x];
    if any(pixel.position.xy < repair_tile.xy)
        || any(pixel.position.xy >= repair_tile.xy + repair_tile.zw) {
        return;
    }
    let xy = pixel.position.xy - repair_tile.xy;
    let index = xy.x + xy.y * repair_tile.z;
    let gain =
        precise_exp(Triple(parameters.anchor.w, parameters.anchor_low.w, parameters.anchor_tail.w));
    var rgb: array<Triple, 3>;
    for(var c = 0u; c < 3u; c++) {
        rgb[c] = precise_mul(number(pixel.value[c]), gain);
    }
    let graded = local_grade(photographic_post(rgb), index);
    let result = finish_pixel(ProcessedPixel(graded, u32(positives[index].w)));
    positives[index] = result;
}

@compute @workgroup_size(64)
fn repair_neutral(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&repair_pixels) {
        return;
    }
    let sample = repair_pixels[id.x];
    let point = sample.position.xy;
    if any(point < repair_tile.xy) || any(point >= repair_tile.xy + repair_tile.zw) {
        return;
    }
    let relative = point - repair_tile.xy;
    let at = relative.x + relative.y * repair_tile.z;
    positives[at] = vec4f(sample.value.xyz, positives[at].w);
}

fn repair_at(key: u32) -> u32 {
    var lo = 0u;
    var hi = arrayLength(&repair_pixels);
    while lo < hi {
        let mid = lo + (hi - lo) / 2u;
        if repair_pixels[mid].position.z < key {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    if lo < arrayLength(&repair_pixels) && repair_pixels[lo].position.z == key {
        return lo;
    }
    return arrayLength(&repair_pixels);
}

// Preserve every other physical pixel's bits, including holes and rotated rectangle corners.
@compute @workgroup_size(8, 8)
fn repair_copy(@builtin(global_invocation_id) id: vec3u) {
    let p = repair_view;
    if any(id.xy >= p.window.zw) {
        return;
    }
    let pixel = id.xy + p.window.xy;
    let start = p.start.xy + f32(pixel.x) * p.dx.xy + f32(pixel.y) * p.dy.xy;
    let points = array<vec2f, 4>(
            start,
            start + p.dx.xy,
            start + p.dx.xy + p.dy.xy,
            start + p.dy.xy,
        );
    var low = points[0];
    var high = low;
    for(var i = 1u; i < 4u; i++) {
        low = min(low, points[i]);
        high = max(high, points[i]);
    }
    let origin = vec2i(p.support.xy) - vec2i(p.bounds_output.xy);
    let lo = max(vec2i(floor(low)), origin);
    let hi = min(vec2i(ceil(high)), origin + vec2i(p.support.zw));
    for(var y = lo.y; y < hi.y; y++) {
        for(var x = lo.x; x < hi.x; x++) {
            let relative = vec2u(vec2i(x, y) - origin);
            if repair_at(relative.x + relative.y * p.support.z) == arrayLength(&repair_pixels) {
                continue;
            }
            if overlap(start, vec2f(f32(x), f32(y)), p.dx.xy, p.dy.xy) <= 0. {
                continue;
            }
            let local = pixel - p.copy_region.xy;
            positives[pixel.x + pixel.y * p.bounds_output.z] = reconstructed[
                    local.x + local.y * p.copy_region.z
                ];
            return;
        }
    }
}
