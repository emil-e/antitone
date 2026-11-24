fn covered_flags(flags: f32, mass: f32, geometry: Geometry) -> f32 {
    let expected = select(
            0.0,
            abs(cross(geometry.start_dx.zw, geometry.dy.xy)),
            geometry.dy.z != 0.0,
        );
    return reconstruction_status(flags, mass, geometry.sizes.x, expected);
}
