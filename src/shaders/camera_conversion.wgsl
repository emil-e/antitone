struct CameraMatrix {
    columns: array<vec4f, 3>,
}
@group(0) @binding(3)
var<uniform> camera_matrix: CameraMatrix;

fn matrix_product(columns: array<vec4f, 3>, rgb: array<f32, 3>) -> array<f32, 3> {
    var output: array<f32, 3>;
    for(var c = 0u; c < 3u; c++) {
        var value = 0.0;
        for(var j = 0u; j < 3u; j++) {
            value = fma(columns[j][c], rgb[j], value);
        }
        output[c] = value;
    }
    return output;
}

fn camera_positive_to_ap1(rgb: array<f32, 3>) -> array<f32, 3> {
    return matrix_product(camera_matrix.columns, rgb);
}
