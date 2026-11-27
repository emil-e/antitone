//! Source descriptions for header inspection, developed pixels and acquisition requests.
use crate::{
    Error,
    project::{Developer, Development, SourceDevelopment},
    raw::{Metadata, RawFile, Rect},
    scan::DevelopedSource,
};
use std::path::Path;

impl SourceDevelopment {
    /// Inspect the expected development without decoding pixels. The returned handle
    /// has been opened with the same WB request that determines the SDK camera basis.
    pub fn read_header(
        path: &Path,
        requested_wb: Option<[f64; 4]>,
    ) -> Result<(RawFile, Self), Error> {
        let raw = RawFile::open_with_wb(path, requested_wb)?;
        let metadata = &raw.metadata;
        let wb = raw.white_balance(requested_wb.unwrap_or_else(|| metadata.as_shot_wb()));
        let (dimensions, active_area) = raw.source_layout()?;

        let development = Self {
            decode: decoder(metadata, wb),
            dimensions,
            active_area: area_fields(active_area),
            camera_model: metadata.camera_model(),
            camera_to_ap1: metadata.camera_to_ap1()?.to_cols_array_2d(),
        };
        Ok((raw, development))
    }

    /// Describe the frozen result using the raster and basis that consumers actually receive.
    pub fn from_developed(source: &DevelopedSource) -> Result<Self, Error> {
        let (width, height) = source.dimensions()?;
        Ok(Self {
            decode: decoder(&source.metadata(), source.white_balance()),
            dimensions: [width, height],
            active_area: area_fields(source.area()),
            camera_model: source.camera_model().to_owned(),
            camera_to_ap1: source.camera_to_ap1().to_cols_array_2d(),
        })
    }

    /// A request targets the installed producer, without rewriting the saved provenance.
    pub(crate) fn for_current_producer(&self) -> Self {
        let mut current = self.clone();
        if let Some(version) = producer_version(current.decode.backend) {
            current.decode.backend_version = version;
        }
        current
    }
}

fn area_fields(area: Rect) -> [u32; 4] {
    [area.x, area.y, area.width, area.height]
}

fn decoder(metadata: &Metadata, white_balance: [f64; 4]) -> Development {
    let backend = if metadata.uses_sdk() {
        Developer::DngSdk
    } else {
        Developer::LibRaw
    };
    Development {
        white_balance,
        backend,
        backend_version: producer_version(backend).expect("opened RAW uses an available decoder"),
    }
}

fn producer_version(backend: Developer) -> Option<String> {
    match backend {
        Developer::LibRaw => Some(crate::raw::development_version()),
        #[cfg(feature = "dng-sdk")]
        Developer::DngSdk => Some(crate::dng::INTERPRETATION.into()),
        #[cfg(not(feature = "dng-sdk"))]
        Developer::DngSdk => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrected_rw2_header_predicts_the_frozen_raster_not_the_sensor_extent() {
        let file = crate::raw::panasonic_fixture::source(
            Some(crate::raw::panasonic_fixture::distortion()),
            true,
        );
        let wb = Some([1.; 4]);
        let (raw, expected) = SourceDevelopment::read_header(file.path(), wb).unwrap();
        assert_eq!(expected.decode.backend, Developer::LibRaw);
        assert!(
            raw.camera_image().is_err(),
            "header inspection does not decode"
        );
        assert_ne!(
            expected.dimensions,
            [raw.metadata.width, raw.metadata.height]
        );
        assert_eq!(expected.active_area, [0, 0, 82, 82]);

        let developed = DevelopedSource::from_raw(raw, wb).unwrap();
        assert!(developed.camera_image().unwrap().is_continuous());
        let actual = SourceDevelopment::from_developed(&developed).unwrap();
        assert_eq!(actual.dimensions, [82; 2]);
        assert_eq!(actual, expected);
    }
}
