fn pixel(p: vec2i, position: vec2f, original: vec4f) -> vec4f {
    if any(p < vec2i(0)) {
        return original;
    }
    let header = table_0(0u);
    let bin = vec2u(p) / header.z;
    let mask = header.x - 1u;
    var slot = ((bin.x * 0x9e3779b1u) ^ (bin.y * 0x85ebca77u)) & mask;
    for(var probe = 0u; probe <= mask; probe += 1u) {
        let entry = table_0(1u + slot);
        if entry.w == 0u {
            return original;
        }
        if all(entry.xy == bin) {
            for(var index = 0u; index < entry.w; index += 1u) {
                let record = header.y + 2u * table_0(entry.z + index).x;
                let rectangle = table_0(record);
                let local = p - vec2i(rectangle.xy);
                if all(local >= vec2i(0)) && all(vec2u(local) < rectangle.zw) {
                    let offset = table_0(record + 1u).x + u32(local.y) * rectangle.z + u32(local.x);
                    let mask = table_0(header.w + offset / 128u);
                    if (mask[(offset / 32u) % 4u] & (1u << (offset % 32u))) != 0u {
                        let replacement = input_1(vec2i(i32(offset), 0));
                        return vec4f(replacement.rgb, original.w);
                    }
                }
            }
            return original;
        }
        slot = (slot + 1u) & mask;
    }
    return original;
}
