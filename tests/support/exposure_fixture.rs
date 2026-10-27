//! Declared neutral camera data with a quantized, independently ACES-rendered exposure reference.
use crate::fixture_path::FixturePath;

#[cfg(feature = "ocio-reference")]
pub fn neutral_dng(exposure: f64) -> FixturePath {
    use crate::rendering::{CpuRenderer, OutputTarget};
    let base = crate::raw::fixture::bayer_dng_with_rgb([6000; 3]);
    let scene = glam::DVec3::splat(6000. / 65535. * exposure.exp2());
    let rendered = CpuRenderer::new(OutputTarget::Srgb)
        .unwrap()
        .render(&[scene])
        .unwrap()[0];
    let code = rendered
        .to_array()
        .map(|v| (OutputTarget::Srgb.encode(f64::from(v)).unwrap() * 255.).round() as u8);
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
        .encode(
            &code
                .into_iter()
                .cycle()
                .take(32 * 32 * 3)
                .collect::<Vec<_>>(),
            32,
            32,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    crate::preview_fixture::append(base.path(), [32, 32], &jpeg, false, Some(2))
}

#[cfg(feature = "ocio-reference")]
pub fn neutral_dng_with_defect(exposure: f64) -> FixturePath {
    let file = neutral_dng(exposure);
    let mut bytes = std::fs::read(file.path()).unwrap();
    assert_eq!(&bytes[..4], b"II\x2a\x00");
    let directory = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(bytes[directory..directory + 2].try_into().unwrap()) as usize;
    let entry = (0..count)
        .map(|i| directory + 2 + i * 12)
        .find(|&entry| u16::from_le_bytes(bytes[entry..entry + 2].try_into().unwrap()) == 273)
        .unwrap();
    assert_eq!(
        u32::from_le_bytes(bytes[entry + 4..entry + 8].try_into().unwrap()),
        1
    );
    let strip = u32::from_le_bytes(bytes[entry + 8..entry + 12].try_into().unwrap()) as usize;
    for y in 12..20 {
        for x in 12..20 {
            let offset = strip + (y * 64 + x) * 2;
            assert_eq!(&bytes[offset..offset + 2], &6000_u16.to_le_bytes());
            bytes[offset..offset + 2].copy_from_slice(&0_u16.to_le_bytes());
        }
    }
    std::fs::write(file.path(), bytes).unwrap();
    file
}
