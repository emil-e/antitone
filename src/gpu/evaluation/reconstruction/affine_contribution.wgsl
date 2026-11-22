fn affine_contribution(p: vec2i, geometry: Geometry) -> Contribution {
    let local = vec2u(p) - geometry.origins.zw;
    let x = affine_axis(local.x);
    let y = affine_axis(geometry.sizes.z + local.y);
    let anchor = bitcast<vec2i>(x.xy) + bitcast<vec2i>(y.xy);
    let start = bitcast<vec2f>(x.zw) + bitcast<vec2f>(y.zw);
    return contribution(geometry, anchor, start);
}
