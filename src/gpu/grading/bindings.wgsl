struct AdjustmentLayerParameters {
    mask_origin: vec2f,
    mask_x_axis: vec2f,
    mask_y_axis: vec2f,
    mask_radii: vec2f,
    wb_log_cone_offsets: vec3f,
    log_exposure_gain: f32,
    mask_feather: f32,
    strength: f32,
    contrast: f32,
    encoded_pivot: f32,
    preserve: f32,
    saturation: f32,
    vibrance: f32,
    shadows_ev: f32,
    highlights_ev: f32,
    mask_kind: u32,
    invert_mask: u32,
    has_wb: u32,
}

struct AdjustmentLayerContext {
    ap1_to_bradford: CameraMatrix,
    bradford_to_ap1: CameraMatrix,
    ap1_to_oklab_lms: CameraMatrix,
    oklab_lms_to_ap1: CameraMatrix,
    oklab_lightness: vec3f,
    inverse_tonal_width: f32,
    ap1_luminance: vec3f,
    encoded_middle_grey: f32,
    source_origin: vec2u,
    source_extent: vec2u,
    layer_count: u32,
}
@group(0) @binding(6)
var<storage, read> adjustment_layer_stages: array<AdjustmentLayerParameters>;
@group(0) @binding(7)
var<uniform> adjustment_layer_context: AdjustmentLayerContext;
