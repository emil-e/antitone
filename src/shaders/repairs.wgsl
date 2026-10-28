struct RepairPixel {
    position: vec4u,
    value: vec4f,
}
@group(0) @binding(12)
var<storage, read> repair_pixels: array<RepairPixel>;
struct RepairTile {
    area: vec4u,
    offset: vec4u,
}
@group(0) @binding(13)
var<uniform> repair_tile: RepairTile;

struct RepairView {
    bounds_output: vec4u,
    support: vec4u,
    window: vec4u,
    copy_region: vec4u,
    start: vec4f,
    dx: vec4f,
    dy: vec4f,
    source: vec4i,
}
@group(0) @binding(14)
var<uniform> repair_view: RepairView;
@group(0) @binding(15)
var<storage, read> reconstructed: array<vec4f>;

fn repair_index(point: vec2u) -> u32 {
    if any(point < repair_tile.area.xy) || any(point >= repair_tile.area.xy + repair_tile.area.zw) {
        return 0xffffffffu;
    }
    let xy = point - repair_tile.area.xy;
    return repair_tile.offset.x + xy.x + xy.y * repair_tile.area.z;
}

fn repaired_positive(value: vec3f, index: u32) -> array<f32, 3> {
    let rgb = value * exp(parameters.anchor.w);
    return adjustment_layer_grade(
            photographic_post(array<f32, 3>(rgb.x, rgb.y, rgb.z), 1.0),
            index,
        );
}

@compute @workgroup_size(64)
fn repair_original(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let repair_cell = id.x + id.y * groups.x * 64u;
    if repair_cell >= arrayLength(&repair_pixels) {
        return;
    }
    let pixel = repair_pixels[repair_cell];
    let index = repair_index(pixel.position.xy);
    if index == 0xffffffffu {
        return;
    }
    let graded = repaired_positive(pixel.value.xyz, index);
    let result = finish_pixel(ProcessedPixel(graded, u32(positives[index].w)));
    positives[index] = result;
}

@compute @workgroup_size(64)
fn repair_neutral(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let repair_cell = id.x + id.y * groups.x * 64u;
    if repair_cell >= arrayLength(&repair_pixels) {
        return;
    }
    let sample = repair_pixels[repair_cell];
    let at = repair_index(sample.position.xy);
    if at == 0xffffffffu {
        return;
    }
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
    let source_high = p.source.xy + p.source.zw;
    let lo = max(clamp(vec2i(floor(low)), p.source.xy, source_high - vec2i(1)), origin);
    let hi = min(
            clamp(vec2i(ceil(high)), p.source.xy + vec2i(1), source_high),
            origin + vec2i(p.support.zw),
        );
    for(var y = lo.y; y < hi.y; y++) {
        for(var x = lo.x; x < hi.x; x++) {
            let relative = vec2u(vec2i(x, y) - origin);
            if repair_at(relative.x + relative.y * p.support.z) == arrayLength(&repair_pixels) {
                continue;
            }
            if extended_overlap(
                    start,
                    vec2f(f32(x), f32(y)),
                    p.dx.xy,
                    p.dy.xy,
                    vec2f(p.source.xy),
                    vec2f(source_high),
                ) <= 0. {
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
