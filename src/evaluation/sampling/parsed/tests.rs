use super::*;

const CODE: &str = "@group(0) @binding(0) var<uniform> a:vec4f; @group(0) @binding(1) var<uniform> b:vec4f; fn first()->f32{return a.x;} fn second()->f32{return b.y;}";

#[test]
fn parsed_programs_reuse_validation_but_select_each_requested_export() {
    let programs = Arc::new(Programs::default());
    let ((first, second), counts) = testing::observe(|| {
        let fragment = Fragment::with_programs(Some(programs.clone()));
        let first = fragment.library(CODE).unwrap();
        assert_eq!(first.resources(&["first"]).unwrap(), [0]);
        let second = fragment.library(CODE).unwrap();
        assert_eq!(second.resources(&["second"]).unwrap(), [1]);
        assert!(matches!(
            second.resources(&["absent"]),
            Err(Error::InvalidParameter(
                "pointwise function is absent from its module"
            ))
        ));
        (first, second)
    });
    assert_eq!(counts, (1, 1));
    assert!(Arc::ptr_eq(&first, &second));

    let (other, counts) = testing::observe(|| {
        let other = Fragment::with_programs(Some(Arc::default()))
            .library(CODE)
            .unwrap();
        assert_eq!(other.resources(&["first", "second"]).unwrap(), [0, 1]);
        other
    });
    assert_eq!(counts, (1, 1));
    assert!(
        !Arc::ptr_eq(&first, &other),
        "independent program owners must not use a global library store"
    );
}

#[test]
fn argument_usage_keeps_live_coordinates_and_reuses_library_validation() {
    let code = "fn unused(p:vec2i, position:vec2f)->vec4f{return vec4f(1.0);} fn helper(position:vec2f)->vec4f{return vec4f(position,0.0,1.0);} fn used(p:vec2i, position:vec2f)->vec4f{return helper(position)+vec4f(f32(p.x));}";
    let (_, counts) = testing::observe(|| {
        let library = Library::parse(code).unwrap();
        assert!(!library.argument_used("unused", 0).unwrap());
        assert!(!library.argument_used("unused", 1).unwrap());
        assert!(library.argument_used("used", 0).unwrap());
        assert!(library.argument_used("used", 1).unwrap());
        assert!(library.resources(&["used"]).unwrap().is_empty());
    });
    assert_eq!(counts, (1, 1));
}

#[test]
fn concurrent_library_compilation_joins_only_the_same_immutable_program() {
    let programs = Arc::new(Programs::default());
    let ready = Arc::new(std::sync::Barrier::new(2));
    let threads = (0..2)
        .map(|_| {
            let programs = programs.clone();
            let ready = ready.clone();
            std::thread::spawn(move || {
                ready.wait();
                testing::observe(|| {
                    let library = programs.libraries.get(CODE).unwrap();
                    assert_eq!(library.resources(&["second"]).unwrap(), [1]);
                    library
                })
            })
        })
        .collect::<Vec<_>>();
    let mut results = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    let (a, (pa, va)) = results.pop().unwrap();
    let (b, (pb, vb)) = results.pop().unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!((pa + pb, va + vb), (1, 1));
}

#[test]
fn invalid_shader_results_do_not_poison_other_programs() {
    let libraries = Libraries::default();
    let (_, counts) = testing::observe(|| {
        assert!(libraries.get("this is not WGSL").is_err());
        assert!(libraries.get("this is not WGSL").is_err());
        let invalid = libraries.get("fn invalid()->i32{return 2u;}").unwrap();
        assert!(matches!(
            invalid.resources(&["invalid"]),
            Err(Error::Gpu(_))
        ));
        assert!(matches!(
            invalid.resources(&["invalid"]),
            Err(Error::Gpu(_))
        ));
        assert_eq!(
            libraries.get(CODE).unwrap().resources(&["second"]).unwrap(),
            [1]
        );
    });
    assert_eq!(counts, (3, 2));
}
