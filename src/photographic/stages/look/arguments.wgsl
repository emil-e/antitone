struct Parameters {
    minimum: vec4f,
    maximum: vec4f,
    reciprocal: vec4f,
    dimensions: vec4u,
}
@group(0) @binding(2)
var<storage, read> nodes: array<vec4f>;
@group(0) @binding(3)
var<uniform> p: Parameters;
