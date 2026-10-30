@group(0) @binding(20)
var<storage, read> native_inputs: array<vec4f>;
@group(0) @binding(21)
var<storage, read_write> native_outputs: array<vec4f>;

@compute @workgroup_size(64)
fn native_response(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&native_inputs) {
        return;
    }
    native_outputs[index] = finish_pixel(interpret_positive(native_inputs[index].rgb));
}
