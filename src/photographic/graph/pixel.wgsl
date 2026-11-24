struct ProcessedPixel {
    rgb: array<f32, 3>,
    flags: u32,
}

fn finish_pixel(pixel: ProcessedPixel) -> vec4f {
    return vec4f(pixel.rgb[0], pixel.rgb[1], pixel.rgb[2], f32(pixel.flags));
}
