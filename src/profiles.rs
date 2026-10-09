use lcms2::{CIExyY, CIExyYTRIPLE, Profile, ToneCurve};

use crate::{Error, rendering::OutputTarget};

pub fn icc(target: OutputTarget) -> &'static [u8] {
    match target {
        OutputTarget::Srgb => include_bytes!("../assets/profiles/antitone-srgb-v1.icc"),
        OutputTarget::DisplayP3 => include_bytes!("../assets/profiles/antitone-display-p3-v1.icc"),
    }
}

/// Regenerates the definition for validation; production embeds the pinned payload above.
pub fn generate(target: OutputTarget) -> Result<Vec<u8>, Error> {
    let p = target.parameters();
    let xy = |x, y| CIExyY { x, y, Y: 1.0 };
    let primaries = CIExyYTRIPLE {
        Red: xy(p[1], p[2]),
        Green: xy(p[3], p[4]),
        Blue: xy(p[5], p[6]),
    };
    // LCMS type 4 is ICC parametric type 3: encoded -> linear, with a linear toe.
    let params = [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045];
    let curve = ToneCurve::new_parametric(4, &params).map_err(color_error)?;
    let mut profile = Profile::new_rgb(&xy(p[7], p[8]), &primaries, &[&curve, &curve, &curve])
        .map_err(color_error)?;
    let description = match target {
        OutputTarget::Srgb => "Antitone sRGB v1",
        OutputTarget::DisplayP3 => "Antitone Display P3 v1",
    };
    let mut text = lcms2::MLU::new(1);
    if !text.set_text(description, lcms2::Locale::new("en_US"))
        || !profile.write_tag(
            lcms2::TagSignature::ProfileDescriptionTag,
            lcms2::Tag::MLU(&text),
        )
    {
        return Err(Error::Rendering("could not write ICC description".into()));
    }
    profile.set_version(4.3);
    profile.set_header_rendering_intent(lcms2::Intent::RelativeColorimetric);
    profile.set_default_profile_id();
    profile.icc().map_err(color_error)
}

pub(crate) fn color_error(error: lcms2::Error) -> Error {
    Error::Rendering(format!("ICC profile: {error}"))
}
