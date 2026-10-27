struct ProcessedPixel {
    rgb: array<f32, 3>,
    flags: u32,
}

const STAGE_SOURCE: u32 = 0u;
const STAGE_NEGATIVE: u32 = 1u;
const STAGE_DIGITAL: u32 = 2u;

fn interpret_positive(scan: vec3f) -> ProcessedPixel {
    if parameters.stage == STAGE_DIGITAL {
        return ProcessedPixel(array<f32, 3>(scan.x, scan.y, scan.z), 0u);
    }
    var rgb: array<f32, 3>;
    var flags = 0u;
    for(var c = 0u; c < 3u; c++) {
        if scan[c] < parameters.dmin_floor.w {
            flags |= 1u << c;
        }
        rgb[c] = invert_film(max(scan[c], parameters.dmin_floor.w), c);
    }
    return ProcessedPixel(rgb, flags);
}
