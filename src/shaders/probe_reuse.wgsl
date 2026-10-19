// Test-only cuts preserve all three words and floor flags; no f32 AP1 round-trip.
@group(0) @binding(8)
var<storage, read_write> response_codes: array<ProcessedPixel>;
@group(0) @binding(9)
var<storage, read_write> prefix_pixels: array<ProcessedPixel>;

fn probe_native_response(scan: ConvertedScan) -> ProcessedPixel {
    var q: array<Triple, 3>;
    var flags = 0u;
    let floor = Triple(parameters.dmin_floor.w, parameters.dmin_low.w, parameters.dmin_tail.w);
    for(var c = 0u; c < 3u; c++) {
        var s = Triple(scan.high[c], scan.low[c], scan.tail[c]);
        if !precise_le(floor, s) {
            flags |= 1u << c;
            s = floor;
        }
        q[c] = invert_film(s, c);
    }
    return ProcessedPixel(q, flags);
}

fn probe_index(id: vec3u, groups: vec3u) -> u32 {
    return id.x + id.y * groups.x * WORKGROUP_SIZE;
}

@compute @workgroup_size(WORKGROUP_SIZE)
fn build_response_codes(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = probe_index(id, groups);
    if index >= arrayLength(&response_codes) {
        return;
    }
    // Every integer input, evaluated by the unchanged GPU operator, including exposure.
    response_codes[index] =
        probe_native_response(convert_camera(vec2u(index | (index << 16u), index)));
}

fn probe_lookup_global(index: u32) -> ProcessedPixel {
    var global: ProcessedPixel;
    let values = source_codes(scans[index], index);
    if field.grid.z != 0u || any(values != floor(values)) || any(values > vec3f(65535.0)) {
        // Fractional samples are not members of the RGB16 code table.
        global = process_film(convert_source(scans[index], index));
    } else {
        let codes = vec3u(values);
        let q = array<Triple, 3>(
                response_codes[codes.x].rgb[0],
                response_codes[codes.y].rgb[1],
                response_codes[codes.z].rgb[2],
            );
        let flags = (response_codes[codes.x].flags & 1u) | (response_codes[codes.y].flags & 2u)
            | (response_codes[codes.z].flags & 4u);
        global = ProcessedPixel(photographic_post(camera_positive_to_ap1(q)), flags);
    }
    return global;
}

@compute @workgroup_size(WORKGROUP_SIZE)
fn lookup_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = probe_index(id, groups);
    if index >= source_pixel_count() {
        return;
    }
    if parameters.gamma_mode.w <= 0.5 {
        positives[index] = process_camera_source(convert_source(scans[index], index));
        return;
    }
    let global = probe_lookup_global(index);
    positives[index] = finish_pixel(ProcessedPixel(local_grade(global.rgb, index), global.flags));
}

@compute @workgroup_size(WORKGROUP_SIZE)
fn build_local_prefix_lookup(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = probe_index(id, groups);
    if index >= source_pixel_count() {
        return;
    }
    let global = probe_lookup_global(index);
    prefix_pixels[index] = ProcessedPixel(local_grade(global.rgb, index), global.flags);
}

@compute @workgroup_size(WORKGROUP_SIZE)
fn build_camera_prefix(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = probe_index(id, groups);
    if index >= source_pixel_count() {
        return;
    }
    let native = probe_native_response(convert_source(scans[index], index));
    prefix_pixels[index] = ProcessedPixel(camera_positive_to_ap1(native.rgb), native.flags);
}

@compute @workgroup_size(WORKGROUP_SIZE)
fn camera_prefix_main(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = probe_index(id, groups);
    if index >= source_pixel_count() {
        return;
    }
    if parameters.gamma_mode.w <= 0.5 {
        positives[index] = process_camera_source(convert_source(scans[index], index));
        return;
    }
    let global = photographic_post(prefix_pixels[index].rgb);
    positives[index] =
        finish_pixel(ProcessedPixel(local_grade(global, index), prefix_pixels[index].flags));
}

@compute @workgroup_size(WORKGROUP_SIZE)
fn build_global_prefix(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = probe_index(id, groups);
    if index >= source_pixel_count() {
        return;
    }
    prefix_pixels[index] = process_film(convert_source(scans[index], index));
}

@compute @workgroup_size(WORKGROUP_SIZE)
fn global_prefix_main(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = probe_index(id, groups);
    if index >= source_pixel_count() {
        return;
    }
    if parameters.gamma_mode.w <= 0.5 {
        positives[index] = process_camera_source(convert_source(scans[index], index));
        return;
    }
    positives[index] = finish_pixel(
            ProcessedPixel(
                local_grade(prefix_pixels[index].rgb, index),
                prefix_pixels[index].flags,
            ),
        );
}
