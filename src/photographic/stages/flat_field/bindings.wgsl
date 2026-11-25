struct FieldGeometry {
    support: vec4u,
    grid: vec4u,
    source: SourceCoordinates,
}
@group(0) @binding(4)
var<storage, read> field_nodes: array<f32>;
@group(0) @binding(5)
var<uniform> field: FieldGeometry;
