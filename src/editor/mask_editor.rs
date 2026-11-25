use super::*;
#[cfg(test)]
use crate::photographic::test_inputs::AuthoredPhotograph;
use crate::processing::InputSettings;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
    geometry::SourcePoint,
    project::{AdjustmentLayerId, AdjustmentLayers},
    viewport::CanvasLayout,
};
use glam::DVec2;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

pub(super) const DEFAULT_RADIAL_FEATHER: f64 = 0.8;

mod hosted;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Linear,
    Radial,
}

impl Kind {
    fn title(self) -> &'static str {
        match self {
            Self::Linear => "Linear gradient",
            Self::Radial => "Radial gradient",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::Linear => {
                "Drag endpoints to change the fade; drag the connector to move it. Release accepts each gesture."
            }
            Self::Radial => {
                "Draw outward from the centre. Drag inside to move, squares to reshape, the outer handle to rotate, or the inner diamond to feather. Release accepts each gesture."
            }
        }
    }
}

enum MaskDrag {
    Linear(LinearDrag),
    Radial(RadialDrag),
}

impl MaskDrag {
    fn update(&self, layout: CanvasLayout, point: egui::Pos2) -> AdjustmentLayerCoverage {
        match self {
            Self::Linear(drag) => drag.update(layout, point),
            Self::Radial(drag) => drag.update(layout, point),
        }
    }
}

/// The tool's target and unfinished gesture; accepted geometry stays in the layer.
pub(super) struct MaskEdit {
    target: Option<AdjustmentLayerId>,
    kind: Kind,
    gesture: Option<MaskGesture>,
    error: Option<String>,
}

struct MaskGesture {
    original: Option<AdjustmentLayerCoverage>,
    mask: Option<AdjustmentLayerCoverage>,
    drag: MaskDrag,
}

#[derive(Clone, Copy)]
enum Handle {
    New,
    Start,
    End,
    Body,
}

struct LinearDrag {
    handle: Handle,
    point: SourcePoint,
    mask: Option<AdjustmentLayerCoverage>,
}

impl MaskEdit {
    pub(super) fn creating(&self) -> bool {
        self.target.is_none()
    }

    #[cfg(test)]
    pub(super) fn valid(&self) -> bool {
        self.gesture
            .as_ref()
            .and_then(|gesture| gesture.mask)
            .is_some_and(|mask| mask.validate().is_ok())
    }

    pub(super) fn dragging(&self) -> bool {
        self.gesture.is_some()
    }

    fn begin_drag(
        &mut self,
        layout: CanvasLayout,
        origin: egui::Pos2,
        mask: Option<AdjustmentLayerCoverage>,
    ) {
        let mask = if self.creating() { None } else { mask };
        let drag = match self.kind {
            Kind::Linear => MaskDrag::Linear(LinearDrag {
                handle: handle_at(mask, layout, origin),
                point: source_point(layout, origin),
                mask,
            }),
            Kind::Radial => MaskDrag::Radial(RadialDrag::begin(
                mask,
                layout,
                origin,
                match mask {
                    Some(AdjustmentLayerCoverage::Radial { feather, .. }) => feather,
                    _ => DEFAULT_RADIAL_FEATHER,
                },
            )),
        };
        self.gesture = Some(MaskGesture {
            original: mask,
            mask,
            drag,
        });
    }

    fn apply(
        &self,
        layers: &mut AdjustmentLayers,
        above: Option<AdjustmentLayerId>,
    ) -> Result<AdjustmentLayerId, Error> {
        let gesture = self
            .gesture
            .as_ref()
            .ok_or(Error::InvalidParameter("draw a gradient first"))?;
        let mask = gesture
            .mask
            .ok_or(Error::InvalidParameter("draw a gradient first"))?;
        mask.validate()?;
        if let Some(id) = self.target {
            let record = layers
                .layers
                .iter_mut()
                .find(|record| record.id == id)
                .ok_or(Error::InvalidParameter("unknown adjustment-layer ID"))?;
            if Some(record.adjustment.mask) != gesture.original {
                return Err(Error::InvalidParameter("mask geometry changed"));
            }
            record.adjustment.mask = mask;
            Ok(id)
        } else {
            layers.add_above(
                above,
                format!(
                    "{} {}",
                    if self.kind == Kind::Linear {
                        "Linear"
                    } else {
                        "Radial"
                    },
                    layers.next_id
                ),
                AdjustmentLayer {
                    mask,
                    photo: AdjustmentLayerSettings::default(),
                    enabled: true,
                    invert: false,
                    strength: 1.,
                },
            )
        }
    }
}

fn source_point(layout: CanvasLayout, point: egui::Pos2) -> SourcePoint {
    layout
        .composition
        .frame_to_source(layout.frame_point(point))
}

fn screen_point(layout: CanvasLayout, point: SourcePoint) -> egui::Pos2 {
    layout.screen_frame_point(layout.composition.source_to_frame(point))
}

fn handle_at(
    mask: Option<AdjustmentLayerCoverage>,
    layout: CanvasLayout,
    point: egui::Pos2,
) -> Handle {
    let Some(AdjustmentLayerCoverage::Linear { start, end }) = mask else {
        return Handle::New;
    };
    let a = screen_point(layout, start);
    let b = screen_point(layout, end);
    if a.distance(point) <= 9. {
        return Handle::Start;
    }
    if b.distance(point) <= 9. {
        return Handle::End;
    }
    let delta = b - a;
    let t = ((point - a).dot(delta) / delta.length_sq()).clamp(0., 1.);
    if (a + delta * t).distance(point) <= 7. {
        Handle::Body
    } else {
        Handle::New
    }
}

impl LinearDrag {
    fn update(&self, layout: CanvasLayout, point: egui::Pos2) -> AdjustmentLayerCoverage {
        let next = source_point(layout, point);
        if next == self.point
            && let Some(mask) = self.mask
        {
            return mask;
        }
        let delta = DVec2::from_array(next.0) - DVec2::from_array(self.point.0);
        match (self.handle, self.mask) {
            (Handle::Start, Some(AdjustmentLayerCoverage::Linear { start, end })) => {
                AdjustmentLayerCoverage::Linear {
                    start: SourcePoint((DVec2::from_array(start.0) + delta).to_array()),
                    end,
                }
            }
            (Handle::End, Some(AdjustmentLayerCoverage::Linear { start, end })) => {
                AdjustmentLayerCoverage::Linear {
                    start,
                    end: SourcePoint((DVec2::from_array(end.0) + delta).to_array()),
                }
            }
            (Handle::Body, Some(AdjustmentLayerCoverage::Linear { start, end })) => {
                AdjustmentLayerCoverage::Linear {
                    start: SourcePoint((DVec2::from_array(start.0) + delta).to_array()),
                    end: SourcePoint((DVec2::from_array(end.0) + delta).to_array()),
                }
            }
            _ => AdjustmentLayerCoverage::Linear {
                start: next,
                end: self.point,
            },
        }
    }
}

fn hits(mask: AdjustmentLayerCoverage, layout: CanvasLayout, point: egui::Pos2) -> bool {
    match mask {
        AdjustmentLayerCoverage::Global {} => false,
        AdjustmentLayerCoverage::Linear { .. } => {
            !matches!(handle_at(Some(mask), layout, point), Handle::New)
        }
        AdjustmentLayerCoverage::Radial { .. } => radial_hit(mask, layout, point),
    }
}

impl Editor {
    pub(super) fn mask_edit(&self) -> Option<Rc<RefCell<MaskEdit>>> {
        self.document
            .visit
            .hosted_tool
            .is::<MaskTool>()
            .then(|| self.document.visit.masks.active.upgrade())
            .flatten()
    }

    #[cfg(test)]
    pub(super) fn mask_creation(&self) -> Option<Rc<RefCell<MaskEdit>>> {
        self.mask_edit().filter(|edit| edit.borrow().creating())
    }

    #[cfg(test)]
    pub(super) fn creating_mask(&self) -> bool {
        self.mask_edit()
            .is_some_and(|edit| edit.borrow().creating())
    }

    pub(super) fn masking(&self) -> bool {
        self.mask_edit().is_some()
    }

    #[cfg(test)]
    pub(super) fn mask_dragging(&self) -> bool {
        self.mask_edit()
            .is_some_and(|edit| edit.borrow().dragging())
    }

    pub(super) fn mask_target(&self) -> Option<AdjustmentLayerId> {
        self.mask_edit().and_then(|edit| edit.borrow().target)
    }

    fn mask_source(&self) -> Result<Source, Error> {
        let mut source = self
            .document
            .project
            .source(
                self.document
                    .visit
                    .current_source
                    .ok_or(Error::InvalidParameter("no selected source"))?,
            )?
            .clone();
        self.overlay_adjustment_layer_settings(&mut source, self.photo_settings());
        source.orientation = self.photo_orientation();
        source.crop = self.photo_crop();
        Ok(source)
    }

    fn selected_mask(&self) -> Option<AdjustmentLayer> {
        let adjustment_layers::Target::Layer(id) = self.adjustment_layer_target() else {
            return None;
        };
        self.document
            .state
            .project
            .source(self.document.visit.current_source?)
            .ok()?
            .adjustment_layers
            .as_ref()?
            .layers
            .iter()
            .find(|record| record.id == id)
            .map(|record| record.adjustment)
            .filter(|layer| layer.mask != (AdjustmentLayerCoverage::Global {}))
    }

    pub(super) fn begin_linear_mask(&mut self, id: Option<AdjustmentLayerId>) {
        self.begin_mask(Kind::Linear, id);
    }

    pub(super) fn begin_radial_mask(&mut self, id: Option<AdjustmentLayerId>) {
        self.begin_mask(Kind::Radial, id);
    }

    pub(super) fn mask_tool_available(&self) -> bool {
        self.tool_start_available()
            && self.selected_area().is_some()
            && self.document.project.input.ready()
    }

    fn begin_mask(&mut self, kind: Kind, id: Option<AdjustmentLayerId>) {
        if !self.mask_tool_available() {
            return;
        }
        let result = (|| {
            let source = self.mask_source()?;
            let mask = id
                .map(|id| {
                    source
                        .adjustment_layers
                        .as_ref()
                        .and_then(|layers| layers.layers.iter().find(|record| record.id == id))
                        .map(|record| record.adjustment.mask)
                        .ok_or(Error::InvalidParameter("unknown adjustment-layer ID"))
                })
                .transpose()?;
            if let Some(mask) = mask {
                let actual = match mask {
                    AdjustmentLayerCoverage::Linear { .. } => Kind::Linear,
                    AdjustmentLayerCoverage::Radial { .. } => Kind::Radial,
                    AdjustmentLayerCoverage::Global {} => {
                        return Err(Error::InvalidParameter(
                            "global layers have no editable mask",
                        ));
                    }
                };
                if actual != kind {
                    return Err(Error::InvalidParameter("mask and tool differ"));
                }
            }
            Ok(mask)
        })();
        if let Err(error) = result {
            self.status = format!("Mask unavailable: {error}");
            return;
        }
        if !self.prepare_tool() {
            return;
        }
        self.document.source = false;
        if self.tab != 1 {
            self.reveal_adjustment_layers();
        }
        let edit = Rc::new(RefCell::new(MaskEdit {
            target: id,
            kind,
            gesture: None,
            error: None,
        }));
        self.document.set_hosted_tool(
            tool::Placement::Photo,
            Box::new(MaskTool::new(edit.clone())),
        );
        self.document.visit.masks.active = Rc::downgrade(&edit);
        if let Some(id) = id {
            self.select_adjustment_layer(adjustment_layers::Target::Layer(id));
        }
        self.status = if id.is_some() {
            format!("Edit {} · Done to finish", kind.title().to_lowercase())
        } else {
            format!("Draw {} · Escape cancels", kind.title().to_lowercase())
        };
    }

    pub(super) fn finish_mask(&mut self, accept: bool) {
        if !self.masking() {
            return;
        }
        if accept {
            self.with_current_tool(|tool, editor| {
                let tool: &mut dyn std::any::Any = tool;
                tool.downcast_mut::<MaskTool>()
                    .expect("active mask tool")
                    .finish(editor);
            });
        } else {
            self.end_tool();
        }
    }

    fn accept_mask(&mut self, edit: &MaskEdit) -> Result<AdjustmentLayerId, Error> {
        let above = match self.adjustment_layer_target() {
            adjustment_layers::Target::Layer(id) => Some(id),
            adjustment_layers::Target::None => None,
        };
        let mut accepted = None;
        self.edit_adjustment_layers(|layers| {
            accepted = Some(edit.apply(layers, above)?);
            Ok(())
        })?;
        Ok(accepted.expect("mask applied"))
    }

    pub(super) fn mask_preview_source(&self) -> Option<Source> {
        if self.tab != 1 {
            return None;
        }
        let active = self.mask_edit()?;
        let edit = active.borrow();
        if edit.creating() || !edit.dragging() {
            return None;
        }
        let mut source = self.mask_source().ok()?;
        edit.apply(
            source
                .adjustment_layers
                .get_or_insert_with(Default::default),
            None,
        )
        .ok()?;
        Some(source)
    }

    pub(super) fn view_recipe(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: AdjustmentLayerSettings,
    ) -> Result<crate::image_recipe::ImageRecipe, Error> {
        let input = input.into();
        if let Some(source) = self.mask_preview_source() {
            self.positive_recipe_for_source(input, roll, &source, photo)
        } else {
            self.positive_recipe(input, roll, photo)
        }
    }

    #[cfg(test)]
    pub(super) fn view_processing(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: AdjustmentLayerSettings,
    ) -> Result<AuthoredPhotograph, Error> {
        let recipe = self.view_recipe(input, roll, photo)?;
        let limits = self
            .document
            .visit
            .working_view
            .device()
            .map(|device| device.limits())
            .unwrap_or_default();
        AuthoredPhotograph::from_recipe(&limits, &recipe)
    }

    #[cfg(test)]
    pub(super) fn test_mask_preview(&mut self, mask: AdjustmentLayerCoverage) {
        let active = self.mask_edit().expect("active mask tool");
        let composition = Composition::with_crop(
            self.selected_area().expect("source area"),
            self.document.project.orientation,
            self.photo_orientation(),
            self.photo_crop(),
        )
        .unwrap();
        let layout = Viewport::default().layout_composition(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.)),
            composition,
            1.,
        );
        let original = self.selected_mask().map(|layer| layer.mask);
        let mut edit = active.borrow_mut();
        edit.begin_drag(layout, layout.image.center(), original);
        edit.gesture.as_mut().expect("started gesture").mask = Some(mask);
    }

    #[cfg(test)]
    pub(super) fn mask_preview_geometry(&self) -> Option<AdjustmentLayerCoverage> {
        self.mask_edit()?.borrow().gesture.as_ref()?.mask
    }

    #[cfg(test)]
    pub(super) fn paint_mask(&self, ui: &egui::Ui, layout: CanvasLayout) {
        if self.document.source || self.tab != 1 {
            return;
        }
        if let Some(edit) = self.mask_edit() {
            edit.borrow().paint(
                ui,
                layout,
                self.selected_mask(),
                self.document.visit.adjustment_layers.show_coverage,
            );
        }
    }
}

fn paint_geometry(
    ui: &egui::Ui,
    layout: CanvasLayout,
    mask: AdjustmentLayerCoverage,
    invert: bool,
    coverage: bool,
    dragging: bool,
) {
    let painter = ui.painter().with_clip_rect(layout.canvas);
    if coverage {
        paint_coverage(
            &painter,
            layout,
            mask,
            invert,
            ui.visuals().selection.stroke.color,
        );
    }
    if let AdjustmentLayerCoverage::Radial { .. } = mask {
        paint_radial(&painter, layout, mask);
        return;
    }
    let AdjustmentLayerCoverage::Linear { start, end } = mask else {
        unreachable!()
    };
    let a = screen_point(layout, start);
    let b = screen_point(layout, end);
    if !a.is_finite() || !b.is_finite() {
        return;
    }
    let delta = b - a;
    if !delta.is_finite() || !delta.length_sq().is_finite() {
        return;
    }
    let stroke = egui::Stroke::new(1., egui::Color32::WHITE);
    painter.line_segment([a, b], stroke);
    if delta.length_sq() > 0. {
        let extent = egui::vec2(-delta.y, delta.x).normalized() * layout.canvas.size().length();
        for point in [a, b] {
            painter.line_segment([point - extent, point + extent], stroke);
        }
        let direction = delta.normalized() * if invert { -1. } else { 1. };
        let tip = a + delta * 0.5 + direction * 7.;
        let side = egui::vec2(-direction.y, direction.x) * 4.;
        painter.add(egui::Shape::convex_polygon(
            vec![
                tip,
                tip - direction * 10. + side,
                tip - direction * 10. - side,
            ],
            egui::Color32::WHITE,
            egui::Stroke::new(1., egui::Color32::BLACK),
        ));
    }
    let hover = ui.input(|input| input.pointer.hover_pos());
    for (point, full) in [(a, invert), (b, !invert)] {
        painter.circle_filled(point, 6., egui::Color32::BLACK);
        painter.circle(
            point,
            4.,
            if full {
                egui::Color32::WHITE
            } else {
                egui::Color32::BLACK
            },
            stroke,
        );
        if dragging || hover.is_some_and(|hover| hover.distance(point) < 15.) {
            let text = if full { "100%" } else { "0%" };
            let galley = painter.layout_no_wrap(
                text.into(),
                egui::FontId::proportional(12.),
                egui::Color32::WHITE,
            );
            let rect =
                egui::Rect::from_min_size(point + egui::vec2(9., -18.), galley.size()).expand(3.);
            painter.rect_filled(rect, 2., egui::Color32::BLACK);
            painter.galley(rect.min + egui::vec2(3., 3.), galley, egui::Color32::WHITE);
        }
    }
}

// Derived UI tessellation only; the photographic mask remains analytic and source-owned.
fn paint_coverage(
    painter: &egui::Painter,
    layout: CanvasLayout,
    mask: AdjustmentLayerCoverage,
    invert: bool,
    color: egui::Color32,
) {
    let rect = layout.image.intersect(layout.canvas);
    if !rect.is_positive() {
        return;
    }
    let columns = ((rect.width() / 8.).ceil() as usize).clamp(1, 160);
    let rows = ((rect.height() / 8.).ceil() as usize).clamp(1, 160);
    let mut mesh = egui::Mesh::default();
    for y in 0..=rows {
        for x in 0..=columns {
            let point = egui::pos2(
                rect.left() + rect.width() * x as f32 / columns as f32,
                rect.top() + rect.height() * y as f32 / rows as f32,
            );
            let coverage = mask.coverage(source_point(layout, point)).unwrap_or(0.);
            let coverage = if invert { 1. - coverage } else { coverage };
            mesh.colored_vertex(point, color.linear_multiply(coverage as f32 * 0.28));
        }
    }
    for y in 0..rows {
        for x in 0..columns {
            let a = (y * (columns + 1) + x) as u32;
            let b = a + columns as u32 + 1;
            mesh.add_triangle(a, a + 1, b);
            mesh.add_triangle(a + 1, b + 1, b);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Weak observation of the photo-owned tool; leaving the tool releases its unfinished mask.
#[derive(Default)]
pub(super) struct MaskEditor {
    active: Weak<RefCell<MaskEdit>>,
}

impl MaskEditor {
    #[cfg(test)]
    pub(super) fn active(&self) -> Option<Rc<RefCell<MaskEdit>>> {
        self.active.upgrade()
    }
}

pub(super) struct MaskTool {
    edit: Rc<RefCell<MaskEdit>>,
}

impl MaskEdit {
    fn cancel_gesture(&mut self) {
        self.gesture = None;
        self.error = None;
    }

    fn update(&mut self, layout: CanvasLayout, point: egui::Pos2) {
        let Some(gesture) = &mut self.gesture else {
            return;
        };
        let mask = gesture.drag.update(layout, point);
        gesture.mask = Some(mask);
        self.error = mask.validate().err().map(|error| error.to_string());
    }

    fn paint(
        &self,
        ui: &egui::Ui,
        layout: CanvasLayout,
        selected: Option<AdjustmentLayer>,
        coverage: bool,
    ) {
        if let Some(mask) = self
            .gesture
            .as_ref()
            .and_then(|gesture| gesture.mask)
            .or_else(|| {
                selected
                    .map(|layer| layer.mask)
                    .filter(|_| !self.creating())
            })
        {
            paint_geometry(
                ui,
                layout,
                mask,
                !self.creating() && selected.is_some_and(|layer| layer.invert),
                coverage,
                self.dragging(),
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RadialHandle {
    New,
    Center,
    Axis(usize, f64),
    Rotate,
    Feather,
}

struct RadialDrag {
    handle: RadialHandle,
    point: SourcePoint,
    mask: Option<AdjustmentLayerCoverage>,
    feather: f64,
}

fn axes(angle: f64) -> [DVec2; 2] {
    let (sin, cos) = angle.sin_cos();
    [DVec2::new(cos, sin), DVec2::new(-sin, cos)]
}

fn position(center: SourcePoint, radii: [f64; 2], angle: f64, unit: DVec2) -> SourcePoint {
    let [x, y] = axes(angle);
    SourcePoint(
        (DVec2::from_array(center.0) + x * radii[0] * unit.x + y * radii[1] * unit.y).to_array(),
    )
}

fn normalized_radius(point: SourcePoint, center: SourcePoint, radii: [f64; 2], angle: f64) -> f64 {
    let delta = DVec2::from_array(point.0) - DVec2::from_array(center.0);
    let [x, y] = axes(angle);
    (delta.dot(x) / radii[0]).hypot(delta.dot(y) / radii[1])
}

fn handles(mask: AdjustmentLayerCoverage, layout: CanvasLayout) -> Vec<(egui::Pos2, RadialHandle)> {
    let AdjustmentLayerCoverage::Radial {
        center,
        radii,
        angle,
        feather,
    } = mask
    else {
        return Vec::new();
    };
    let screen = screen_point(layout, center);
    let x_end = screen_point(layout, position(center, radii, angle, DVec2::X));
    let x_direction = x_end - screen;
    let mut result = vec![(screen, RadialHandle::Center)];
    for axis in 0..2 {
        for sign in [-1., 1.] {
            let unit = if axis == 0 {
                DVec2::new(sign, 0.)
            } else {
                DVec2::new(0., sign)
            };
            result.push((
                screen_point(layout, position(center, radii, angle, unit)),
                RadialHandle::Axis(axis, sign),
            ));
        }
    }
    if x_direction.is_finite()
        && x_direction.length_sq().is_finite()
        && x_direction.length_sq() > 0.
    {
        result.push((x_end + x_direction.normalized() * 28., RadialHandle::Rotate));
    }
    // At full feather the inner boundary collapses; the slider remains available.
    if feather < 1. {
        let diagonal = DVec2::splat(std::f64::consts::FRAC_1_SQRT_2 * (1. - feather));
        result.push((
            screen_point(layout, position(center, radii, angle, diagonal)),
            RadialHandle::Feather,
        ));
    }
    result
}

fn radial_hit(mask: AdjustmentLayerCoverage, layout: CanvasLayout, point: egui::Pos2) -> bool {
    radial_handle_at(Some(mask), layout, point) != RadialHandle::New
}

fn radial_handle_at(
    mask: Option<AdjustmentLayerCoverage>,
    layout: CanvasLayout,
    point: egui::Pos2,
) -> RadialHandle {
    let Some(
        mask @ AdjustmentLayerCoverage::Radial {
            center,
            radii,
            angle,
            ..
        },
    ) = mask
    else {
        return RadialHandle::New;
    };
    if radii
        .iter()
        .any(|radius| !radius.is_finite() || *radius <= 0.)
    {
        return RadialHandle::New;
    }
    if let Some((_, handle)) = handles(mask, layout)
        .into_iter()
        .filter(|(position, _)| position.is_finite() && position.distance(point) <= 9.)
        .min_by(|(a, _), (b, _)| a.distance_sq(point).total_cmp(&b.distance_sq(point)))
    {
        return handle;
    }
    if normalized_radius(source_point(layout, point), center, radii, angle) < 1. {
        RadialHandle::Center
    } else {
        RadialHandle::New
    }
}

impl RadialDrag {
    fn begin(
        mask: Option<AdjustmentLayerCoverage>,
        layout: CanvasLayout,
        point: egui::Pos2,
        feather: f64,
    ) -> Self {
        Self {
            handle: radial_handle_at(mask, layout, point),
            point: source_point(layout, point),
            mask,
            feather,
        }
    }

    fn update(&self, layout: CanvasLayout, point: egui::Pos2) -> AdjustmentLayerCoverage {
        let next = source_point(layout, point);
        if next == self.point
            && let Some(mask) = self.mask
        {
            return mask;
        }
        if let Some(AdjustmentLayerCoverage::Radial {
            mut center,
            mut radii,
            mut angle,
            mut feather,
        }) = self.mask
            && self.handle != RadialHandle::New
        {
            let delta = DVec2::from_array(next.0) - DVec2::from_array(self.point.0);
            match self.handle {
                RadialHandle::Center => {
                    center = SourcePoint((DVec2::from_array(center.0) + delta).to_array())
                }
                RadialHandle::Axis(axis, sign) => {
                    radii[axis] = (radii[axis] + sign * delta.dot(axes(angle)[axis])).abs()
                }
                RadialHandle::Rotate => {
                    let before = DVec2::from_array(self.point.0) - DVec2::from_array(center.0);
                    let after = DVec2::from_array(next.0) - DVec2::from_array(center.0);
                    if before.length_squared() > 0. && after.length_squared() > 0. {
                        let rotation = after.y.atan2(after.x) - before.y.atan2(before.x);
                        angle += rotation.sin().atan2(rotation.cos());
                    }
                }
                RadialHandle::Feather => {
                    let before = normalized_radius(self.point, center, radii, angle);
                    let after = normalized_radius(next, center, radii, angle);
                    feather = (feather + before - after).clamp(0., 1.);
                }
                RadialHandle::New => unreachable!(),
            }
            return AdjustmentLayerCoverage::Radial {
                center,
                radii,
                angle,
                feather,
            };
        }
        let start = layout.composition.source_to_frame(self.point);
        let end = layout.composition.source_to_frame(next);
        let horizontal = layout
            .composition
            .frame_to_source(crate::geometry::FramePoint([start.0[0] + 1., start.0[1]]));
        let direction = DVec2::from_array(horizontal.0) - DVec2::from_array(self.point.0);
        AdjustmentLayerCoverage::Radial {
            center: self.point,
            radii: [(end.0[0] - start.0[0]).hypot(end.0[1] - start.0[1]); 2],
            angle: direction.y.atan2(direction.x),
            feather: self.feather,
        }
    }
}

fn paint_radial(painter: &egui::Painter, layout: CanvasLayout, mask: AdjustmentLayerCoverage) {
    let AdjustmentLayerCoverage::Radial {
        center,
        radii,
        angle,
        feather,
    } = mask
    else {
        return;
    };
    let screen = screen_point(layout, center);
    let x_end = screen_point(layout, position(center, radii, angle, DVec2::X));
    let y_end = screen_point(layout, position(center, radii, angle, DVec2::Y));
    if ![screen, x_end, y_end].iter().all(|point| point.is_finite()) {
        return;
    }
    let radius = screen.distance(x_end).max(screen.distance(y_end));
    if !radius.is_finite() {
        return;
    }
    // This bounds UI-guide tessellation only; photographic masks remain analytic.
    let segments = (std::f32::consts::PI * (radius / 0.5).sqrt())
        .ceil()
        .clamp(32., 1024.) as usize;
    let stroke = egui::Stroke::new(1., egui::Color32::WHITE);
    for scale in [1., 1. - feather] {
        if scale == 0. {
            continue;
        }
        let points: Vec<_> = (0..=segments)
            .map(|index| {
                let theta = std::f64::consts::TAU * index as f64 / segments as f64;
                screen_point(
                    layout,
                    position(
                        center,
                        radii,
                        angle,
                        DVec2::new(theta.cos(), theta.sin()) * scale,
                    ),
                )
            })
            .collect();
        if points.iter().all(|point| point.is_finite()) {
            painter.add(egui::Shape::line(points, stroke));
        }
        if feather == 0. {
            break;
        }
    }
    for (point, handle) in handles(mask, layout) {
        if !point.is_finite() {
            continue;
        }
        match handle {
            RadialHandle::Axis(..) => {
                painter.rect(
                    egui::Rect::from_center_size(point, egui::vec2(7., 7.)),
                    0.,
                    egui::Color32::BLACK,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
            RadialHandle::Feather => {
                let points = [
                    egui::vec2(0., -5.),
                    egui::vec2(5., 0.),
                    egui::vec2(0., 5.),
                    egui::vec2(-5., 0.),
                ]
                .map(|delta| point + delta)
                .to_vec();
                painter.add(egui::Shape::convex_polygon(
                    points,
                    egui::Color32::BLACK,
                    stroke,
                ));
            }
            RadialHandle::Center | RadialHandle::Rotate => {
                if handle == RadialHandle::Rotate {
                    painter.line_segment([x_end, point], stroke);
                }
                painter.circle_filled(point, 4., egui::Color32::BLACK);
                painter.circle_stroke(point, 4., stroke);
            }
            RadialHandle::New => {}
        }
    }
}

#[cfg(test)]
mod contextual_tests;
#[cfg(test)]
mod radial_tests;
#[cfg(test)]
mod save_tests;
#[cfg(test)]
mod tests;
