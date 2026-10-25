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

fn view_coefficient(high: f32, low: f32, tail: f32) -> f32 {
    return high + (low + tail);
}

fn view_point(index: u32) -> vec2f {
    let xy = vec2f(f32(index % view.dimensions.x), f32(index / view.dimensions.x));
    let origin = vec2f(
        view_coefficient(view.origin[0][0], view.origin[1][0], view.origin[2][0]),
        view_coefficient(view.origin[0][1], view.origin[1][1], view.origin[2][1]),
    );
    let dx = vec2f(
        view_coefficient(view.axis_x[0][0], view.axis_x[1][0], view.axis_x[2][0]),
        view_coefficient(view.axis_x[0][1], view.axis_x[1][1], view.axis_x[2][1]),
    );
    let dy = vec2f(
        view_coefficient(view.axis_y[0][0], view.axis_y[1][0], view.axis_y[2][0]),
        view_coefficient(view.axis_y[0][1], view.axis_y[1][1], view.axis_y[2][1]),
    );
    return origin + xy.x * dx + xy.y * dy;
}
