use super::*;
use std::mem::{offset_of, size_of};

#[test]
fn grading_members_match_host_offsets() {
    let module = naga::front::wgsl::parse_str(&Grading::shader()).unwrap();

    macro_rules! check_members {
        ($host:ty, $($field:ident),+ $(,)?) => {{
            let name = stringify!($host);
            let (_, shader) = module.types.iter().find(|(_, ty)| ty.name.as_deref() == Some(name))
                .expect("matching shader record");
            let naga::TypeInner::Struct { members, span } = &shader.inner else {
                panic!("{name} must be a shader struct");
            };
            let expected = [$( (stringify!($field), offset_of!($host, $field)) ),+];
            assert_eq!(members.len(), expected.len(), "{name}");
            assert_eq!(*span as usize, size_of::<$host>(), "{name}");
            for (member, (field, offset)) in members.iter().zip(expected) {
                assert_eq!(member.name.as_deref(), Some(field), "{name}");
                assert_eq!(member.offset as usize, offset, "{name}.{field}");
            }
        }};
    }

    check_members!(
        AdjustmentLayerParameters,
        mask_origin,
        mask_x_axis,
        mask_y_axis,
        mask_radii,
        wb_log_cone_offsets,
        log_exposure_gain,
        mask_feather,
        strength,
        contrast,
        encoded_pivot,
        preserve,
        saturation,
        vibrance,
        shadows_ev,
        highlights_ev,
        mask_kind,
        invert_mask,
        has_wb,
    );
    check_members!(
        AdjustmentLayerContext,
        ap1_to_bradford,
        bradford_to_ap1,
        ap1_to_oklab_lms,
        oklab_lms_to_ap1,
        oklab_lightness,
        inverse_tonal_width,
        ap1_luminance,
        encoded_middle_grey,
        source_origin,
        source_extent,
        layer_count,
    );
}
