struct WorkingView {
    dimensions: vec4u,
    origin: vec4f,
    axis_x: vec4f,
    axis_y: vec4f,
    gain: vec4f,
}
@group(0) @binding(10)
var<uniform> view: WorkingView;
@group(0) @binding(11)
var<storage, read> working_source: array<vec4f>;

fn view_point(index: u32) -> vec2f {
    let xy = vec2f(f32(index % view.dimensions.x), f32(index / view.dimensions.x));
    return view.origin.xy + xy.x * view.axis_x.xy + xy.y * view.axis_y.xy;
}
