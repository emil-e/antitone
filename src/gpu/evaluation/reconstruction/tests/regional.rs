use super::*;
use std::sync::Mutex;

struct AnalyticPlane {
    blue: u32,
    reads: Arc<Mutex<Vec<(u32, Rect)>>>,
}
impl Source for AnalyticPlane {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 1024, 256].into()),
            grid: Grid {
                coverage: 8,
                ..Grid::axis_aligned([0.; 2], [1.; 2])
            },
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("analytic mosaic plane", self.blue)))
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.lock().unwrap().push((self.blue, region));
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right()).map(move |x| {
                    [
                        1. + x as f32 / 256.,
                        -0.5 - y as f32 / 256.,
                        0.25 + self.blue as f32,
                        8.,
                    ]
                })
            })
            .collect())
    }
}

// An ordinary rectangular collage: no healing, masks or repair-specific definitions.
struct Mosaic(Rect);
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}
impl Operation for Mosaic {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("rectangular mosaic", self.0)))
    }
    fn describe(
        &self,
        output: Output,
        inputs: &[Value],
        requested: &[Rect],
    ) -> Result<Output, Error> {
        let inset = requested.iter().any(|r| overlaps(*r, self.0));
        let base = requested.iter().any(|r| !self.0.contains_region(*r));
        if !inset {
            return inputs[0].output(output.region);
        }
        if !base {
            return inputs[1].output(output.region);
        }
        Ok(Output::calculated(
            output.domain,
            output.region,
            self.key(),
            inputs.iter().map(Value::content),
        ))
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: Rect,
        _: Budget,
    ) -> Result<Option<value::Calculation>, Error> {
        let calculation = if !overlaps(output, self.0) {
            value::Calculation::input(inputs[0].request(output)?)
        } else if self.0.contains_region(output) {
            value::Calculation::input(inputs[1].request(output)?)
        } else {
            value::Calculation::new(domain, output,
                inputs.iter().map(|input| input.request(output).map(value::Calculation::input)).collect::<Result<_, _>>()?,
                value::Operation::new("rectangular mosaic", self.0,
                    "fn mosaic(p:vec2i,at:vec2f,rect:vec4i)->vec4f { if all(p>=rect.xy) && all(p<rect.zw) { return input_1(p); } return input_0(p); }",
                    "mosaic", value::Access::Neighborhood)
                    .with_uniform("vec4i", &[self.0.x,self.0.y,self.0.right(),self.0.bottom()]),
            )?
        };
        Ok(Some(calculation))
    }
}
fn mosaic(blue: u32, inset: Rect, reads: &Arc<Mutex<Vec<(u32, Rect)>>>) -> Value {
    let planes = [0, blue].map(|blue| {
        source(Arc::new(AnalyticPlane {
            blue,
            reads: reads.clone(),
        }))
    });
    Value::from_operation(planes[0].domain(), planes, Mosaic(inset))
}

#[test]
fn tiled_reconstruction_describes_disjoint_dependencies_without_knowing_their_operation() {
    let reads = Arc::new(Mutex::new(Vec::new()));
    let first = mosaic(1, [16, 20, 1, 1].into(), &reads);
    let changed = mosaic(2, [16, 20, 1, 1].into(), &reads);
    let domain = Domain {
        bounds: None,
        grid: Grid {
            origin: [20., 20.],
            axes: [[1., 1.], [-1., 1.]],
            pitch: [2f64.sqrt(); 2],
            coverage: 8,
        },
        meaning: Meaning::Neutral,
    };
    let a = reconstruct_tiled(first.clone(), domain).unwrap();
    let b = reconstruct_tiled(changed.clone(), domain).unwrap();
    let requested = Rect::new([0, 0], [4, 4]);
    let enclosing: Rect = [16, 20, 8, 8].into();
    assert_ne!(
        first.output(enclosing).unwrap(),
        changed.output(enclosing).unwrap(),
        "the changed inset really lies inside the bounding rectangle"
    );
    assert!(matches!(
        first.content_for(enclosing, &[[0, 0, 1, 1].into()]),
        Err(Error::InvalidParameter(
            "described cells exceed the requested image region"
        ))
    ));
    let (_, definitions) = value::testing::observe(0, || {
        let (_, calculations) = value::testing::observe_calculations(0, || {
            assert_eq!(
                a.output(requested).unwrap(),
                b.output(requested).unwrap(),
                "the inset at the diamond's bounding-box corner contributes no cells"
            );
            assert_ne!(
                a.output([-4, -4, 12, 12].into()).unwrap(),
                b.output([-4, -4, 12, 12].into()).unwrap(),
                "an expanded diamond reaches the inset and must notice its changed pixels"
            );
        });
        assert_eq!(calculations, 0);
    });
    assert_eq!(definitions, 0);
    assert!(reads.lock().unwrap().is_empty());
}

#[test]
fn regional_content_does_not_hide_a_captured_dependency() {
    let reads = Arc::new(Mutex::new(Vec::new()));
    let domain = Domain {
        bounds: None,
        grid: Grid {
            coverage: 8,
            ..Grid::axis_aligned([0.; 2], [2.; 2])
        },
        meaning: Meaning::Neutral,
    };
    let make =
        |blue| reconstruct_tiled(mosaic(blue, [512, 0, 512, 256].into(), &reads), domain).unwrap();
    let complete = make(1);
    let region: Rect = [0, 0, 64, 32].into();
    let requested = complete.request(region).unwrap();
    assert!(complete.same_capture(&requested));
    assert_ne!(
        requested.identity(),
        complete.identity(),
        "the selected panel really narrows regional content"
    );
    let wrapper = Value::new(
        Domain {
            bounds: Some(region),
            meaning: Meaning::Graded,
            ..domain
        },
        region,
        vec![requested.clone()],
        value::Operation::new(
            "regional dependency witness",
            (),
            "fn pixel(p:vec2i,at:vec2f,a:vec4f)->vec4f{return a;}",
            "pixel",
            value::Access::Pointwise,
        ),
    )
    .unwrap();
    assert_eq!(
        wrapper.region_for(&complete, region).unwrap(),
        Some(region),
        "a requested operand must still identify its complete mathematical ancestor"
    );
    let (regions, calculations) = value::testing::observe_calculations(0, || {
        requested
            .regions_for(
                &complete,
                region,
                Budget {
                    tile: [64, 32],
                    scratch_bytes: 1 << 20,
                },
            )
            .unwrap()
    });
    assert_eq!(regions, vec![region]);
    assert_eq!(
        calculations, 0,
        "a captured ancestor match must stop before expanding its implementation"
    );
    let equivalent = make(1);
    assert!(!complete.same_capture(&equivalent));
    assert!(
        requested.same_definition(&equivalent),
        "equivalence is not limited to pointer identity"
    );
    assert_eq!(
        wrapper.region_for(&equivalent, region).unwrap(),
        Some(region)
    );
    let changed = make(2);
    assert_eq!(
        requested.output(region).unwrap(),
        changed.output(region).unwrap(),
        "changing the other panel leaves these requested pixels unchanged"
    );
    assert!(
        !requested.same_definition(&changed),
        "regional equality cannot erase different complete definitions"
    );
    assert_eq!(
        wrapper.region_for(&changed, region).unwrap(),
        None,
        "equal requested pixels do not turn an unrelated definition into an ancestor"
    );
    let other_region: Rect = [256, 0, 64, 32].into();
    assert_ne!(
        complete.output(other_region).unwrap(),
        changed.output(other_region).unwrap()
    );
    assert!(complete.same_definition(&complete.request(other_region).unwrap()));
    assert!(
        reads.lock().unwrap().is_empty(),
        "dependency discovery must not acquire pixels"
    );
}

#[test]
fn a_common_grid_reuses_actual_tiles_across_mosaic_edits_and_compatible_panning() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 32 << 20,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let reads = Arc::new(Mutex::new(Vec::new()));
    let make = |blue| {
        reconstruct_tiled(
            mosaic(blue, [512, 0, 512, 256].into(), &reads),
            Domain {
                bounds: None,
                grid: Grid {
                    coverage: 8,
                    ..Grid::axis_aligned([0.; 2], [2.; 2])
                },
                meaning: Meaning::Neutral,
            },
        )
        .unwrap()
    };
    let materialize = |root: &Value, region: Rect| {
        let query = context
            .request(
                root.clone(),
                Region {
                    bounds: region,
                    grid: root.domain().grid,
                },
                Budget {
                    tile: [region.width, region.height],
                    scratch_bytes: 2 << 20,
                },
            )
            .unwrap();
        pollster::block_on(
            context
                .materialize(
                    &queue,
                    &Default::default(),
                    &scope,
                    query,
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap()
    };
    let tile = |root: &Value| {
        let region: Rect = [128, 0, 128, 128].into();
        scope
            .find_ready::<ImageLayout>(root.output(region).unwrap().identity().unwrap(), |layout| {
                layout.region == region
            })
            .expect("the overlapping GPU tile must actually exist")
    };
    let check = |image: &Image<ImageLayout>, blue, phase: f32| {
        let pixels =
            crate::gpu::evaluation::execute::read(&device, &queue, image.as_buffer().unwrap());
        let region = image.descriptor.region;
        assert_eq!(pixels.len(), region.width as usize * region.height as usize);
        for y in region.y..region.bottom() {
            for x in region.x..region.right() {
                let inset_mass = ((2. * x as f32 + phase + 2. - 512.) / 2.).clamp(0., 1.);
                let expected = [
                    1. + (2. * x as f32 + 0.5 + phase) / 256.,
                    -0.5 - (2. * y as f32 + 0.5) / 256.,
                    0.25 + blue as f32 * inset_mass,
                    8.,
                ];
                let actual = pixels[region.index([x, y])];
                assert_eq!(actual[3], expected[3]);
                for c in 0..3 {
                    assert!(
                        (actual[c] - expected[c]).abs() < 2e-6,
                        "analytic 2x2 mean differs at {x},{y}: {actual:?} vs {expected:?}"
                    );
                }
            }
        }
    };
    let first = make(1);
    let region: Rect = [0, 0, 384, 128].into();
    let cold = materialize(&first, region);
    check(&cold, 1, 0.);
    let before = tile(&first);
    assert!(before.as_buffer().is_some());
    assert!(
        reads
            .lock()
            .unwrap()
            .iter()
            .any(|(blue, r)| *blue == 0 && r.x < 512)
    );
    assert!(
        reads
            .lock()
            .unwrap()
            .iter()
            .any(|(blue, r)| *blue == 1 && r.x >= 512)
    );
    reads.lock().unwrap().clear();
    let changed = make(2);
    let edited = materialize(&changed, region);
    check(&edited, 2, 0.);
    assert!(Arc::ptr_eq(&before.image, &tile(&changed).image));
    {
        let acquired = reads.lock().unwrap();
        assert!(!acquired.is_empty(), "the changed panel must execute");
        assert!(
            acquired.iter().all(|(blue, r)| *blue == 2 && r.x >= 512),
            "unchanged panel reread: {acquired:?}"
        );
    }
    reads.lock().unwrap().clear();
    let panned = materialize(&changed, [128, 0, 384, 128].into());
    check(&panned, 2, 0.);
    assert!(Arc::ptr_eq(&before.image, &tile(&changed).image));
    let acquired = reads.lock().unwrap();
    assert!(
        !acquired.is_empty(),
        "panning must execute the newly exposed region"
    );
    assert!(
        acquired.iter().all(|(blue, r)| *blue == 2 && r.x >= 768),
        "overlapping reconstructed regions reread: {acquired:?}"
    );
    drop(acquired);
    let mut shifted_domain = changed.domain();
    shifted_domain.grid.origin[0] = 0.25;
    let shifted =
        reconstruct_tiled(mosaic(2, [512, 0, 512, 256].into(), &reads), shifted_domain).unwrap();
    let shifted_region = [128, 0, 128, 128].into();
    assert_ne!(
        changed.output(shifted_region).unwrap(),
        shifted.output(shifted_region).unwrap(),
        "a fractional sampling phase is different mathematics, not a compatible pan"
    );
    let fractional = materialize(&shifted, shifted_region);
    check(&fractional, 2, 0.25);
    assert!(!Arc::ptr_eq(&before.image, &tile(&shifted).image));
    drop((cold, edited, panned, fractional));
}
