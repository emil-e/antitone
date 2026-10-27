// Exact affine pixel/cell overlap, shared by source acquisition and reference reconstruction.
struct Polygon {
    points: array<vec2f, 8>,
    count: u32,
}

fn cross(a: vec2f, b: vec2f) -> f32 {
    return a.x * b.y - a.y * b.x;
}

fn inside(v: vec2f, axis: u32, edge: f32, greater: bool) -> bool {
    return select(v[axis] <= edge, v[axis] >= edge, greater);
}

fn clipped(poly: Polygon, axis: u32, edge: f32, greater: bool) -> Polygon {
    var result: Polygon;
    if poly.count == 0u {
        return result;
    }
    for(var i = 0u; i < poly.count; i++) {
        let a = poly.points[(i + poly.count - 1u) % poly.count];
        let b = poly.points[i];
        let ia = inside(a, axis, edge, greater);
        let ib = inside(b, axis, edge, greater);
        if ia != ib && a[axis] != edge && b[axis] != edge {
            var point = a + (b - a) * ((edge - a[axis]) / (b[axis] - a[axis]));
            point[axis] = edge;
            result.points[result.count] = point;
            result.count++;
        }
        if ib {
            result.points[result.count] = b;
            result.count++;
        }
    }
    return result;
}

fn overlap(start: vec2f, cell: vec2f, dx: vec2f, dy: vec2f) -> f32 {
    let determinant = cross(dx, dy);
    var full = true;
    for(var i = 0u; i < 4u; i++) {
        let corner =
            cell + array<vec2f, 4>(vec2f(0., 0.), vec2f(1., 0.), vec2f(1., 1.), vec2f(0., 1.))[i]
            - start;
        let local = vec2f(cross(corner, dy), cross(dx, corner)) / determinant;
        full = full && all(local >= vec2f(0.)) && all(local <= vec2f(1.));
    }
    if full {
        return 1.;
    }
    var poly: Polygon;
    poly.count = 4u;
    poly.points[0] = start - cell;
    poly.points[1] = start + dx - cell;
    poly.points[2] = start + dx + dy - cell;
    poly.points[3] = start + dy - cell;
    poly = clipped(poly, 0u, 0., true);
    poly = clipped(poly, 0u, 1., false);
    poly = clipped(poly, 1u, 0., true);
    poly = clipped(poly, 1u, 1., false);
    if poly.count < 3u {
        return 0.;
    }
    var area = 0.;
    for(var i = 1u; i + 1u < poly.count; i++) {
        area += cross(poly.points[i] - poly.points[0], poly.points[i + 1u] - poly.points[0]);
    }
    return min(abs(area) * 0.5, 1.);
}

// The outermost real cells extend constantly beyond the source, never beyond a tile boundary.
fn extended_overlap(
    start: vec2f,
    cell: vec2f,
    dx: vec2f,
    dy: vec2f,
    source_low: vec2f,
    source_high: vec2f,
) -> f32 {
    let points = array<vec2f, 4>(start, start + dx, start + dx + dy, start + dy);
    var low = points[0];
    var high = low;
    for(var i = 1u; i < 4u; i++) {
        low = min(low, points[i]);
        high = max(high, points[i]);
    }
    let cell_low = select(cell, min(cell, low), cell == source_low);
    let cell_high = select(
            cell + vec2f(1.),
            max(cell + vec2f(1.), high),
            cell + vec2f(1.) == source_high,
        );
    if all(cell_low == cell) && all(cell_high == cell + vec2f(1.)) {
        return overlap(start, cell, dx, dy);
    }
    var poly: Polygon;
    poly.count = 4u;
    for(var i = 0u; i < 4u; i++) {
        poly.points[i] = points[i] - cell_low;
    }
    let extent = cell_high - cell_low;
    poly = clipped(poly, 0u, 0., true);
    poly = clipped(poly, 0u, extent.x, false);
    poly = clipped(poly, 1u, 0., true);
    poly = clipped(poly, 1u, extent.y, false);
    if poly.count < 3u {
        return 0.;
    }
    var area = 0.;
    for(var i = 1u; i + 1u < poly.count; i++) {
        area += cross(poly.points[i] - poly.points[0], poly.points[i + 1u] - poly.points[0]);
    }
    return abs(area) * 0.5;
}
