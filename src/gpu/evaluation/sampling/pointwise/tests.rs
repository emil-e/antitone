use super::*;
fn function() -> Function {
    Function::new(
        "@group(0) @binding(2) var<uniform> parameters:vec4f; fn pixel(p:vec4f, position:vec2f)->vec4f{return p*parameters;}",
        "pixel",
    )
}
fn linked(function: Function, prefix: &str, first: u32) -> Result<Fragment, Error> {
    let mut fragment = Fragment::default();
    let mut slot = first;
    let (_, bindings) = link(function, &["pixel"], prefix, &mut slot, &mut fragment)?;
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].0, first);
    assert_eq!(slot, first + 1);
    Ok(fragment)
}
#[test]
fn linked_code_is_shared_but_each_plan_captures_its_current_parameters() {
    let template = function();
    let a = linked(
        template.clone().with_uniform(2, &[1_f32, 2., 3., 4.]),
        "test_",
        64,
    )
    .unwrap();
    let first = template.linked.lock().unwrap().clone().unwrap();
    let b = linked(
        template.clone().with_uniform(2, &[4_f32, 3., 2., 1.]),
        "test_",
        64,
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        &first,
        template.linked.lock().unwrap().as_ref().unwrap()
    ));
    for (fragment, expected) in [(a, [1_f32, 2., 3., 4.]), (b, [4_f32, 3., 2., 1.])] {
        let Storage::Uniform(bytes) = &fragment.resources[0] else {
            panic!("captured uniform")
        };
        assert_eq!(bytemuck::cast_slice::<u8, f32>(bytes), expected);
    }
    assert!(matches!(
        linked(template.clone(), "test_", 64),
        Err(Error::InvalidParameter(
            "pointwise calculation lacks a captured resource"
        ))
    ));
    assert!(matches!(
        linked(
            template
                .clone()
                .with_uniform(2, &[0_f32; 4])
                .with_uniform(3, &[0_f32; 4]),
            "test_",
            64
        ),
        Err(Error::InvalidParameter(
            "pointwise capture has no shader resource"
        ))
    ));
}
#[test]
fn namespace_slots_symbols_and_source_code_requalify_the_single_linked_variant() {
    let mut template = function();
    linked(template.clone().with_uniform(2, &[1_f32; 4]), "first_", 64).unwrap();
    let first = template.linked.lock().unwrap().clone().unwrap();
    linked(template.clone().with_uniform(2, &[1_f32; 4]), "second_", 64).unwrap();
    let namespace = template.linked.lock().unwrap().clone().unwrap();
    assert_ne!(first.code, namespace.code);
    linked(template.clone().with_uniform(2, &[1_f32; 4]), "second_", 70).unwrap();
    let second = template.linked.lock().unwrap().clone().unwrap();
    assert_ne!(first.code, second.code);
    assert!(!Arc::ptr_eq(&first, &second));
    assert_ne!(namespace.code, second.code);
    let second_code = second.code.clone();
    let weak = Arc::downgrade(&second);
    drop(second);
    template.code="@group(0) @binding(2) var<uniform> parameters:vec4f; fn pixel(p:vec4f, position:vec2f)->vec4f{return p+parameters;}".into();
    linked(template.clone().with_uniform(2, &[1_f32; 4]), "second_", 70).unwrap();
    let third = template.linked.lock().unwrap().clone().unwrap();
    assert_ne!(third.code, second_code);
    assert!(
        weak.upgrade().is_none(),
        "the code owner keeps only its most recent linked variant"
    );
    let mut fragment = Fragment::default();
    let mut slot = 70;
    assert!(matches!(
        link(
            template.with_uniform(2, &[1_f32; 4]),
            &["absent"],
            "second_",
            &mut slot,
            &mut fragment
        ),
        Err(Error::InvalidParameter(
            "pointwise function is absent from its module"
        ))
    ));
}
