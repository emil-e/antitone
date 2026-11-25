struct ImageMtf {
    raster: Raster,
    raster_sigma: vec4f,
    radii: vec4u,
}
@group(0) @binding(11)
var<storage, read> working_source: array<vec4f>;
@group(0) @binding(10)
var<uniform> parameters: ImageMtf;
