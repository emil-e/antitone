struct WorkingView {
    dimensions: vec4u,
    origin: array<vec4f, 3>,
    axis_x: array<vec4f, 3>,
    axis_y: array<vec4f, 3>,
    gain: vec4f,
}
@group(0) @binding(10)
var<uniform> view: WorkingView;
@group(0) @binding(11)
var<storage, read> working_source: array<vec4f>;

fn view_point(index: u32) -> array<Triple, 2> {
    let xy = vec2f(f32(index % view.dimensions.x), f32(index / view.dimensions.x));
    var point: array<Triple, 2>;
    for(var c = 0u; c < 2u; c++) {
        let origin = Triple(view.origin[0][c], view.origin[1][c], view.origin[2][c]);
        let dx = Triple(view.axis_x[0][c], view.axis_x[1][c], view.axis_x[2][c]);
        let dy = Triple(view.axis_y[0][c], view.axis_y[1][c], view.axis_y[2][c]);
        point[c] = precise_add(
                origin,
                precise_add(precise_mul(number(xy.x), dx), precise_mul(number(xy.y), dy)),
            );
    }
    return point;
}
