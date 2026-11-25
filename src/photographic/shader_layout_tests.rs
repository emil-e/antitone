use crate::photographic::stages::{
    flat_field::{self, FieldGeometry},
    grading::{AdjustmentLayerContext, AdjustmentLayerParameters},
    shader::functions,
};
use crate::{
    acquisition::camera::coordinates::{self, SourceCoordinates},
    photographic::parameters::{CameraMatrix, ProcessingParameters},
};
use naga::{AddressSpace, TypeInner};
use std::mem::size_of;

#[test]
fn correction_and_grading_use_separate_uniforms_with_the_same_source_mapping() {
    for (shader, name, bytes) in [
        (flat_field::shader(), "field", size_of::<FieldGeometry>()),
        (
            coordinates::shader(),
            "source_coordinates",
            size_of::<SourceCoordinates>(),
        ),
    ] {
        let module = naga::front::wgsl::parse_str(&shader).unwrap();
        let (_, uniform) = module
            .global_variables
            .iter()
            .find(|(_, global)| global.name.as_deref() == Some(name))
            .unwrap();
        assert_eq!(
            uniform.binding,
            Some(naga::ResourceBinding {
                group: 0,
                binding: 5
            })
        );
        assert_eq!(uniform.space, AddressSpace::Uniform);
        let TypeInner::Struct { span, members } = &module.types[uniform.ty].inner else {
            panic!("{name} must be a packed uniform structure");
        };
        assert_eq!(*span as usize, bytes);
        if name == "field" {
            let source = members
                .iter()
                .find(|member| member.name.as_deref() == Some("source"))
                .unwrap();
            assert_eq!(source.offset, 32);
            assert_eq!(
                module.types[source.ty].name.as_deref(),
                Some("SourceCoordinates")
            );
        }
    }
}

#[test]
fn photographic_resource_bindings_and_sizes_match_host() {
    let module = naga::front::wgsl::parse_str(&functions()).unwrap();
    let global = |name| {
        module
            .global_variables
            .iter()
            .find_map(|(_, value)| (value.name.as_deref() == Some(name)).then_some(value))
            .unwrap()
    };
    for (name, binding, bytes) in [
        ("parameters", 2, size_of::<ProcessingParameters>()),
        ("camera_matrix", 3, size_of::<CameraMatrix>()),
        (
            "adjustment_layer_context",
            7,
            size_of::<AdjustmentLayerContext>(),
        ),
    ] {
        let value = global(name);
        assert_eq!(
            value.binding,
            Some(naga::ResourceBinding { group: 0, binding }),
            "{name}"
        );
        assert_eq!(value.space, AddressSpace::Uniform, "{name}");
        let TypeInner::Struct { span, .. } = &module.types[value.ty].inner else {
            panic!("{name} must retain its packed uniform structure");
        };
        assert_eq!(*span as usize, bytes, "{name}");
    }
    let layers = global("adjustment_layer_stages");
    assert_eq!(
        layers.binding,
        Some(naga::ResourceBinding {
            group: 0,
            binding: 6
        })
    );
    assert_eq!(
        layers.space,
        AddressSpace::Storage {
            access: naga::StorageAccess::LOAD
        }
    );
    let TypeInner::Array { stride, .. } = &module.types[layers.ty].inner else {
        panic!("adjustment layers must retain their packed storage array");
    };
    assert_eq!(*stride as usize, size_of::<AdjustmentLayerParameters>());
}
