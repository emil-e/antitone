@group(0) @binding(8)
var<storage, read> response_codes: array<ProcessedPixel>;
@group(0) @binding(9)
var<storage, read_write> response_destination: array<ProcessedPixel>;

@compute @workgroup_size(64)
fn build_response_codes(@builtin(global_invocation_id) id: vec3u) {
    let code = id.x;
    if code >= arrayLength(&response_destination) {
        return;
    }
    response_destination[code] = native_response(convert_camera(vec2u(code | (code << 16u), code)));
}

fn process_positive(code: vec2u, index: u32) -> ProcessedPixel {
    if field.grid.z != 0u {
        // Correction produces continuous values, not integer table indices.
        return process_film(convert_source(code, index));
    }
    let codes = vec3u(code.x & 65535u, code.x >> 16u, code.y & 65535u);
    let q = array<Triple, 3>(
            response_codes[codes.x].rgb[0],
            response_codes[codes.y].rgb[1],
            response_codes[codes.z].rgb[2],
        );
    let flags = (response_codes[codes.x].flags & 1u) | (response_codes[codes.y].flags & 2u)
        | (response_codes[codes.z].flags & 4u);
    return ProcessedPixel(photographic_post(camera_positive_to_ap1(q)), flags);
}
