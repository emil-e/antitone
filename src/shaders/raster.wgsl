struct Raster {
    dimensions: vec4u,
    origin: vec4f,
    axis_x: vec4f,
    axis_y: vec4f,
}

fn raster_point(raster: Raster, index: u32) -> vec2f {
    let xy = vec2f(f32(index % raster.dimensions.x), f32(index / raster.dimensions.x));
    return raster.origin.xy + xy.x * raster.axis_x.xy + xy.y * raster.axis_y.xy;
}

fn raster_covered(raster: Raster, pixel: vec4f) -> bool {
    let mask = raster.dimensions.z;
    return mask == 0u || (u32(pixel.w) & mask) != 0u;
}

fn raster_index(id: vec3u, groups: vec3u) -> u32 {
    return id.x + id.y * groups.x * 64u;
}
