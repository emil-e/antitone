@group(0) @binding(9)
var<uniform> neutral_readout_gain: vec4f;

fn decode_neutral_readout(pixel: vec4f, position: vec2f) -> vec4f {
    return vec4f(pixel.rgb * neutral_readout_gain.x, pixel.w);
}
