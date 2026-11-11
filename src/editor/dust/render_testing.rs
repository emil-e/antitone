//! UI-test observations use ordinary healing definitions, never a second evaluation cache.
use super::*;
use crate::dust_repair::BaseImage;
#[cfg(feature = "ui-tests")]
use crate::gpu::evaluation::Value;

pub(super) fn pixels(
    scan: &Scan,
    recipe: &crate::image_recipe::ImageRecipe,
    repair: &Repair,
    services: &source::Services,
) -> Vec<([u32; 2], [f32; 3])> {
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    let base = crate::gpu::repair_source::Source::from_recipe(
        &services.device,
        &renderer,
        (&services.queue, services.renderers.submissions()),
        scan,
        recipe,
    )
    .unwrap();
    let image = crate::gpu::repairs::definition::healing_image(base.image(), repair).unwrap();
    let mut reader = crate::gpu::repair_source::Source::from_image(
        &renderer,
        (&services.queue, services.renderers.submissions()),
        image,
    )
    .unwrap();
    let [x, y] = repair.support.origin();
    let [width, height] = repair.support.size();
    let values = reader
        .read(
            SourceRegion {
                x,
                y,
                width,
                height,
            },
            None,
            &|| false,
        )
        .unwrap()
        .unwrap();
    repair
        .support
        .selected_points()
        .map(|point| {
            (
                point,
                values[((point[1] - y) * width + point[0] - x) as usize],
            )
        })
        .collect()
}

#[cfg(feature = "ui-tests")]
pub(in crate::editor) struct Image {
    pub definition: Value,
    repair: Repair,
    services: source::Services,
}
#[cfg(feature = "ui-tests")]
impl std::ops::Deref for Image {
    type Target = Repair;
    fn deref(&self) -> &Repair {
        &self.repair
    }
}
#[cfg(feature = "ui-tests")]
impl Image {
    pub fn same_definition(&self, other: &Self) -> bool {
        self.definition.same_definition(&other.definition)
    }
    pub fn support(&self) -> &SupportMask {
        &self.repair.support
    }
    // This performs common graph work: call outside an Editor/Harness borrow.
    pub fn pixels(&self) -> std::vec::IntoIter<([u32; 2], [f32; 3])> {
        let services = &self.services;
        let renderer = services
            .renderers
            .get(&services.device, &services.queue)
            .unwrap();
        let mut source = crate::gpu::repair_source::Source::from_image(
            &renderer,
            (&services.queue, services.renderers.submissions()),
            self.definition.clone(),
        )
        .unwrap();
        let [x, y] = self.repair.support.origin();
        let [width, height] = self.repair.support.size();
        let region = SourceRegion {
            x,
            y,
            width,
            height,
        };
        let values = source.read(region, None, &|| false).unwrap().unwrap();
        self.repair
            .support
            .selected_points()
            .map(|point| {
                let index = ((point[1] - y) * width + point[0] - x) as usize;
                (point, values[index])
            })
            .collect::<Vec<_>>()
            .into_iter()
    }
}

#[cfg(feature = "ui-tests")]
impl Editor {
    pub(in crate::editor) fn dust_images(&self) -> Vec<Arc<Image>> {
        let capture = self
            .positive_recipe(
                self.input(),
                self.document.project_snapshot().roll,
                self.photo_settings(),
            )
            .unwrap();
        let ready = capture
            .repairs
            .iter()
            .filter(|repair| {
                repair.enabled
                    && self.dust_repair_feedback(&capture, repair) == RepairFeedback::Ready
            })
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Vec::new();
        }
        let scan = self.source_context().unwrap().scan().unwrap();
        let (scan, capture) = crate::photo_preparation::native_image(&scan, &capture).unwrap();
        let graph = crate::gpu::photograph::RecipeGraph::originals(
            crate::gpu::evaluation::camera_source(&scan).unwrap(),
            self.document
                .correction
                .prepared()
                .map(|correction| correction.gpu.clone()),
            &capture,
        )
        .unwrap();
        let base = graph.interpreted().unwrap().unrepaired;
        ready
            .into_iter()
            .map(|repair| {
                Arc::new(Image {
                    definition: crate::gpu::repairs::definition::healing_image(&base, repair)
                        .unwrap(),
                    repair: repair.clone(),
                    services: self.source_services.clone().expect("image-query services"),
                })
            })
            .collect()
    }
}
