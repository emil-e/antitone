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
