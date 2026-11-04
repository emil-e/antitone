use super::*;
use crate::gpu::evaluation::{Image, ImageBacking, ImageFormat, ImageLayout, Materializations};
use crate::gpu::photograph::request::PhotographRequest;

pub(super) fn materialize(
    renderer: &crate::renderer::GpuRenderer,
    queue: &wgpu::Queue,
    scope: &Materializations,
    image: Arc<ImageNode>,
    bounds: CellRegion,
) -> Image<ImageLayout> {
    let context = renderer.evaluation();
    let evaluation = context
        .request(
            image.clone(),
            Region {
                bounds,
                grid: image.domain().grid,
            },
            Budget {
                tile: [bounds.width, bounds.height],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    pollster::block_on(
        context
            .materialize(
                queue,
                &Default::default(),
                scope,
                evaluation,
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn changed_looks_request_a_new_photograph_but_reuse_the_upstream_gpu_product() {
    use crate::looks::{LookAsset, LookOrigin, Lut3D};
    use crate::rendering::OutputReference;
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (mut project, id) = project();
    project.input = crate::processing::InputSettings::Digital;
    let plain_recipe = Arc::new(project.image_recipe(id).unwrap());
    let codes = evaluation::source(Arc::new(CameraCodes));
    let plain = RecipeGraph::originals(codes.clone(), None, &plain_recipe, &Default::default())
        .unwrap()
        .photograph(&device.limits())
        .unwrap();
    let scope = renderer.evaluation().test_materializations(false);
    let bounds = plain.processed.domain().finite().unwrap();
    let first = materialize(&renderer, &queue, &scope, plain.processed.clone(), bounds);
    let initial_pixels = evaluation::read_buffer(&device, &queue, first.as_buffer().unwrap());

    let lut = Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.4, 0.3, 0.2]; 8]).unwrap());
    project.output.lut = Some(Arc::new(
        LookAsset::new("Look".into(), LookOrigin::Imported, lut).unwrap(),
    ));
    let looked_recipe = Arc::new(project.image_recipe(id).unwrap());
    let request = PhotographRequest::Recipe(looked_recipe.clone());
    assert!(PhotographRequest::Recipe(plain_recipe.clone()) != request);
    let looked = RecipeGraph::originals(codes.clone(), None, &looked_recipe, &Default::default())
        .unwrap()
        .photograph(&device.limits())
        .unwrap();
    let upstream = materialize(&renderer, &queue, &scope, looked.processed.clone(), bounds);
    assert!(Arc::ptr_eq(&first.image, &upstream.image));

    let display = OutputReference::DisplayReferred(crate::rendering::OutputTarget::Srgb);
    let plain_output =
        evaluation::Reference::capture(&device, &renderer, display, &plain_recipe.luts)
            .unwrap()
            .compose(plain.processed, &renderer)
            .unwrap();
    let looked_output = evaluation::Reference::capture(&device, &renderer, display, request.luts())
        .unwrap()
        .compose(looked.processed.clone(), &renderer)
        .unwrap();
    assert!(!plain_output.same_definition(&looked_output));
    assert_ne!(
        read(&renderer, &queue, plain_output),
        read(&renderer, &queue, looked_output)
    );

    let before_looks = PhotographRequest::BeforeLooks(looked_recipe.clone());
    assert!(PhotographRequest::BeforeLooks(plain_recipe) != before_looks);
    let bypassed = evaluation::Reference::capture(&device, &renderer, display, before_looks.luts())
        .unwrap()
        .looked(looked.processed);
    let bypassed = materialize(&renderer, &queue, &scope, bypassed, bounds);
    assert!(Arc::ptr_eq(&first.image, &bypassed.image));

    // Control: upstream changes must not inherit the downstream-only reuse above.
    project.roll.exposure = 1.;
    let changed_recipe = project.image_recipe(id).unwrap();
    let changed = RecipeGraph::originals(codes, None, &changed_recipe, &Default::default())
        .unwrap()
        .photograph(&device.limits())
        .unwrap();
    assert!(plain.neutral.same_definition(&changed.neutral));
    let changed = materialize(&renderer, &queue, &scope, changed.processed, bounds);
    assert!(!Arc::ptr_eq(&first.image, &changed.image));
    let changed_pixels = evaluation::read_buffer(&device, &queue, changed.as_buffer().unwrap());
    for (initial, changed) in initial_pixels.iter().zip(changed_pixels) {
        for channel in 0..3 {
            assert!((changed[channel] - 2. * initial[channel]).abs() < 2e-6);
        }
    }
}

#[test]
fn disabled_effect_edits_are_captured_without_replacing_an_unchanged_gpu_calculation() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (mut project, id) = project();
    project.input = crate::processing::InputSettings::Digital;
    let before_recipe = project.image_recipe(id).unwrap();
    let codes = evaluation::source(Arc::new(CameraCodes));
    let before = RecipeGraph::originals(codes.clone(), None, &before_recipe, &Default::default())
        .unwrap()
        .photograph(&device.limits())
        .unwrap();
    let scope = renderer.evaluation().test_materializations(false);
    let bounds = before.processed.domain().finite().unwrap();
    let before = materialize(&renderer, &queue, &scope, before.processed, bounds);
    project.output.halation.strength = 0.4;
    project.output.grain.amount = 0.1;
    let after_recipe = project.image_recipe(id).unwrap();
    assert_ne!(before_recipe, after_recipe);
    let after = RecipeGraph::originals(codes, None, &after_recipe, &Default::default())
        .unwrap()
        .photograph(&device.limits())
        .unwrap();
    let after = materialize(&renderer, &queue, &scope, after.processed, bounds);
    assert!(Arc::ptr_eq(&before.image, &after.image));
}
