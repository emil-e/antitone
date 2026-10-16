struct FieldGeometry {
    support: vec4u,
    grid: vec4u,
    area: vec4u,
    dimensions: vec4u,
    packed_origins: vec4u,
}
@group(0) @binding(4)
var<storage, read> field_nodes: array<f32>;
@group(0) @binding(5)
var<uniform> field: FieldGeometry;

// Division and bilinear steps match the scalar f64 rounding, as camera conversion does.
fn divide_integer(a: Triple, b: f32) -> Triple {
    let h = a.h / b;
    let r = add(a, multiply(Triple(-h, 0.0, 0.0), Triple(b, 0.0, 0.0)));
    let l = (r.h + r.l) / b;
    let s = add(r, multiply(Triple(-l, 0.0, 0.0), Triple(b, 0.0, 0.0)));
    let t = (s.h + s.l + s.t) / b;
    return round53(Triple(h, l, t));
}

fn negate(a: Triple) -> Triple {
    return Triple(-a.h, -a.l, -a.t);
}

fn field_node(index: u32, channel: u32) -> Triple {
    let i = 9u * index + channel;
    return Triple(field_nodes[i], field_nodes[i + 3u], field_nodes[i + 6u]);
}

fn field_lerp(a: Triple, b: Triple, t: Triple) -> Triple {
    let one_minus = round53(add(Triple(1.0, 0.0, 0.0), negate(t)));
    return round53(add(round53(multiply(a, one_minus)), round53(multiply(b, t))));
}

fn source_pixel(index: u32) -> vec2u {
    if field.dimensions.z != 0u {
        let count = field.dimensions.z * field.dimensions.w;
        let tile = index / count;
        let local = index % count;
        let offset = 2u * tile;
        return vec2u(field.packed_origins[offset], field.packed_origins[offset + 1u])
            + vec2u(local % field.dimensions.z, local / field.dimensions.z);
    }
    let xy = vec2u(index % field.dimensions.x, index / field.dimensions.x);
    return field.area.xy + ((2u * xy + vec2u(1u)) * field.area.zw) / (2u * field.dimensions.xy);
}

fn field_gains(index: u32) -> array<Triple, 3> {
    let source = source_pixel(index);
    let identity = Triple(1.0, 0.0, 0.0);
    if any(source < field.support.xy) || any(source >= field.support.xy + field.support.zw) {
        return array<Triple, 3>(identity, identity, identity);
    }
    let relative = source - field.support.xy;
    let xx = round53(
            multiply(
                divide_integer(
                    add_term(Triple(f32(relative.x), 0.0, 0.0), 0.5),
                    f32(field.support.z),
                ),
                Triple(f32(field.grid.x - 1u), 0.0, 0.0),
            ),
        );
    let yy = round53(
            multiply(
                divide_integer(
                    add_term(Triple(f32(relative.y), 0.0, 0.0), 0.5),
                    f32(field.support.w),
                ),
                Triple(f32(field.grid.y - 1u), 0.0, 0.0),
            ),
        );
    // A rounded high part can land just above an integer when the complete value is below it.
    let x0 = min(
            u32(floor(xx.h)) - select(0u, 1u, xx.h == floor(xx.h) && xx.l < 0.0),
            field.grid.x - 2u,
        );
    let y0 = min(
            u32(floor(yy.h)) - select(0u, 1u, yy.h == floor(yy.h) && yy.l < 0.0),
            field.grid.y - 2u,
        );
    let dx = round53(add(xx, Triple(-f32(x0), 0.0, 0.0)));
    let dy = round53(add(yy, Triple(-f32(y0), 0.0, 0.0)));
    let start = y0 * field.grid.x + x0;
    var gains: array<Triple, 3>;
    for(var c = 0u; c < 3u; c++) {
        let a = field_lerp(field_node(start, c), field_node(start + 1u, c), dx);
        let b = field_lerp(
                field_node(start + field.grid.x, c),
                field_node(start + field.grid.x + 1u, c),
                dx,
            );
        gains[c] = field_lerp(a, b, dy);
    }
    return gains;
}

fn convert_source(code: vec2u, index: u32) -> ConvertedScan {
    if field.grid.z == 0u {
        return convert_camera(code);
    }
    let gains = field_gains(index);
    let x = round53(multiply(normalized(code.x & 65535u), gains[0]));
    let y = round53(multiply(normalized(code.x >> 16u), gains[1]));
    let z = round53(multiply(normalized(code.y & 65535u), gains[2]));
    return convert_components(x, y, z);
}
