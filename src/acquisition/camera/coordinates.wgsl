struct SourceCoordinates {
    area: vec4u,
    dimensions: vec4u,
    packed_origins: vec4u,
}

fn source_pixel(coordinates: SourceCoordinates, index: u32) -> vec2u {
    if coordinates.dimensions.z != 0u {
        let count = coordinates.dimensions.z * coordinates.dimensions.w;
        let tile = index / count;
        let local = index % count;
        let offset = 2u * tile;
        return vec2u(coordinates.packed_origins[offset], coordinates.packed_origins[offset + 1u])
            + vec2u(local % coordinates.dimensions.z, local / coordinates.dimensions.z);
    }
    let xy = vec2u(index % coordinates.dimensions.x, index / coordinates.dimensions.x);
    return coordinates.area.xy
        + ((2u * xy + vec2u(1u)) * coordinates.area.zw) / (2u * coordinates.dimensions.xy);
}
