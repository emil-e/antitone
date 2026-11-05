use super::*;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::processing::InputSettings;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
    geometry::SourcePoint,
    project::AdjustmentLayerId,
    viewport::CanvasLayout,
};
use glam::DVec2;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

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
                "Drag endpoints to change the fade; drag the connector to move it. Release accepts; Escape cancels this gesture."
            }
            Self::Radial => {
                "Draw outward from the centre. Drag inside to move, squares to reshape, the outer handle to rotate, or the inner diamond to feather. Release accepts; Escape cancels this gesture."
            }
        }
    }
}

enum MaskDrag {
    Linear(LinearDrag),
    Radial(RadialDrag),
}

impl MaskDrag {
    fn update(&self, point: egui::Pos2) -> AdjustmentLayerCoverage {
        match self {
            Self::Linear(drag) => drag.update(point),
            Self::Radial(drag) => drag.update(point),
        }
    }
}

/// Captured geometry and inputs shared by creation and existing-handle gestures.
pub(super) struct MaskEdit {
    loaded: u64,
    source: Source,
    roll: RollSettings,
    input: InputSettings,
    id: Option<AdjustmentLayerId>,
    above: Option<AdjustmentLayerId>,
    mask: Option<AdjustmentLayerCoverage>,
    preview: Option<Source>,
    kind: Kind,
    feather: f64,
    drag: Option<MaskDrag>,
    error: Option<String>,
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
    layout: CanvasLayout,
}

impl MaskEdit {
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(super) fn creating(&self) -> bool {
        self.id.is_none()
    }

    #[cfg(test)]
    pub(super) fn valid(&self) -> bool {
        self.mask.is_some_and(|mask| mask.validate().is_ok())
    }

    pub(super) fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    fn begin_drag(&mut self, layout: CanvasLayout, origin: egui::Pos2) {
        self.drag = Some(match self.kind {
            Kind::Linear => MaskDrag::Linear(LinearDrag {
                handle: if self.creating() {
                    Handle::New
                } else {
                    handle_at(self.mask, layout, origin)
                },
                point: source_point(layout, origin),
                mask: self.mask,
                layout,
            }),
            Kind::Radial => MaskDrag::Radial(RadialDrag::begin(
                if self.creating() { None } else { self.mask },
                layout,
                origin,
                self.feather,
            )),
        });
    }

    fn candidate(&self) -> Result<(Source, AdjustmentLayerId), Error> {
        let mask = self
            .mask
            .ok_or(Error::InvalidParameter("draw a gradient first"))?;
        mask.validate()?;
        let mut source = self.source.clone();
        let layers = source
            .adjustment_layers
            .get_or_insert_with(Default::default);
        let id = if let Some(id) = self.id {
            let record = layers
                .layers
                .iter_mut()
                .find(|record| record.id == id)
                .ok_or(Error::InvalidParameter("unknown adjustment-layer ID"))?;
            record.adjustment.mask = mask;
            id
        } else {
            layers.add_above(
                self.above,
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
            )?
        };
        Ok((source, id))
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
    fn update(&self, point: egui::Pos2) -> AdjustmentLayerCoverage {
        let next = source_point(self.layout, point);
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
    #[cfg(test)]
    pub(super) fn mask_creation(&self) -> Option<Rc<RefCell<MaskEdit>>> {
        self.document.visit.masks.creation.upgrade()
    }

    pub(super) fn creating_mask(&self) -> bool {
        self.document.visit.masks.creation.upgrade().is_some()
    }

    pub(super) fn masking(&self) -> bool {
        self.creating_mask() || self.document.visit.masks.editing()
    }

    pub(super) fn mask_dragging(&self) -> bool {
        self.document.visit.masks.dragging()
            || self
                .document
                .visit
                .masks
                .creation
                .upgrade()
                .is_some_and(|edit| edit.borrow().dragging())
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
        self.overlay_adjustment_layer_settings(
            &mut source,
            self.document.visit.adjustment_layer_settings,
        );
        source.orientation = self.document.visit.photo_orientation;
        source.crop = self.document.visit.crop;
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

    pub(super) fn mask_help(&self) -> Option<&'static str> {
        if self.tab != 1 {
            return None;
        }
        self.document
            .visit
            .masks
            .appearance(self.selected_mask())
            .map(|layer| match layer.mask {
                AdjustmentLayerCoverage::Global {} => {
                    unreachable!("global layers have no mask handles")
                }
                AdjustmentLayerCoverage::Linear { .. } => Kind::Linear,
                AdjustmentLayerCoverage::Radial { .. } => Kind::Radial,
            })
            .map(Kind::help)
    }

    pub(super) fn mask_handles_available(&self) -> bool {
        self.adjustment_layer_overlays_visible()
            && !self.has_hosted_tool()
            && !self.document.visit.adjustment_layers.dragging()
            && self.tab == 1
            && !self.interaction_blocked()
            && self.selected_area().is_some()
            && self.document.input.ready()
            && !self.document.source
            && !self.picking()
            && !self.document.visit.adjustment_layers.confirming()
            && self.fullscreen.is_none()
    }

    pub(super) fn mask_pointer_interest(&self, layout: CanvasLayout, point: egui::Pos2) -> bool {
        self.mask_handles_available()
            && self
                .document
                .visit
                .masks
                .pointer_interest(self.selected_mask(), layout, point)
    }

    pub(super) fn begin_linear_mask(&mut self, id: Option<AdjustmentLayerId>) {
        self.begin_mask(Kind::Linear, id);
    }

    pub(super) fn begin_radial_mask(&mut self, id: Option<AdjustmentLayerId>) {
        self.begin_mask(Kind::Radial, id);
    }

    fn begin_mask(&mut self, kind: Kind, id: Option<AdjustmentLayerId>) {
        if !self.tool_start_available()
            || id.is_some() && self.has_hosted_tool()
            || self.selected_area().is_none()
            || !self.document.input.ready()
            || self.fullscreen.is_some()
        {
            return;
        }
        if id.is_none() && !self.prepare_tool() {
            return;
        }
        self.document.source = false;
        let result = (|| {
            let source = self.mask_source()?;
            self.validate_source_candidate(&source)?;
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
            if mask.is_some_and(|mask| {
                matches!(mask, AdjustmentLayerCoverage::Linear { .. }) != (kind == Kind::Linear)
            }) {
                return Err(Error::InvalidParameter("mask and tool differ"));
            }
            Ok((source, mask))
        })();
        match result {
            Ok((source, mask)) => {
                if !(self.tab == 1) {
                    self.reveal_adjustment_layers();
                }
                if let Some(id) = id {
                    self.select_adjustment_layer(adjustment_layers::Target::Layer(id));
                }
                let edit = MaskEdit {
                    loaded: self.id,
                    source,
                    roll: self.document.roll,
                    input: self.document.input,
                    id,
                    above: match self.adjustment_layer_target() {
                        adjustment_layers::Target::Layer(id) => Some(id),
                        adjustment_layers::Target::None => None,
                    },
                    mask,
                    preview: None,
                    kind,
                    feather: match mask {
                        Some(AdjustmentLayerCoverage::Radial { feather, .. }) => feather,
                        _ => controls::DEFAULT_RADIAL_FEATHER,
                    },
                    drag: None,
                    error: None,
                };
                if edit.creating() {
                    let edit = Rc::new(RefCell::new(edit));
                    self.document.visit.masks.creation = Rc::downgrade(&edit);
                    self.document
                        .set_hosted_tool(tool::Placement::Photo, Box::new(CreationTool::new(edit)));
                } else {
                    self.document.visit.masks.begin(edit);
                }
                self.status = format!("Draw {} · Escape cancels", kind.title().to_lowercase());
            }
            Err(error) => self.status = format!("Mask unavailable: {error}"),
        }
    }

    fn mask_is_current(&self, edit: &MaskEdit) -> bool {
        edit.loaded == self.id
            && self.mask_source().ok().as_ref() == Some(&edit.source)
            && self.document.roll == edit.roll
            && self.document.input == edit.input
            && edit.id.is_none_or(|id| {
                self.adjustment_layer_target() == adjustment_layers::Target::Layer(id)
            })
    }

    pub(super) fn finish_mask(&mut self, accept: bool) {
        let Some(edit) = self.document.visit.masks.finish() else {
            return;
        };
        if !accept || !(self.tab == 1) {
            self.status = "Mask gesture cancelled".into();
            return;
        }
        if !self.mask_is_current(&edit) {
            self.status = "Mask gesture discarded: source or settings changed".into();
            return;
        }
        let result = edit
            .candidate()
            .and_then(|candidate| self.accept_mask(candidate));
        match result {
            Ok(id) => {
                self.select_adjustment_layer(adjustment_layers::Target::Layer(id));
                self.status = format!("{} selected", edit.kind.title());
            }
            Err(error) => {
                self.status = format!("Mask rejected: {error}");
            }
        }
    }

    fn accept_mask(
        &mut self,
        (candidate, id): (Source, AdjustmentLayerId),
    ) -> Result<AdjustmentLayerId, Error> {
        self.validate_source_candidate(&candidate)?;
        self.edit_adjustment_layers(|layers| {
            *layers = candidate
                .adjustment_layers
                .expect("candidate has local records");
            Ok(())
        })?;
        Ok(id)
    }

    pub(super) fn mask_preview_source(&self) -> Option<&Source> {
        if self.fullscreen.is_some() || self.tab != 1 {
            return None;
        }
        self.document
            .visit
            .masks
            .gesture()
            .filter(|edit| self.mask_is_current(edit))
            .and_then(|edit| edit.preview.as_ref())
    }

    pub(super) fn view_recipe(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: AdjustmentLayerSettings,
    ) -> Result<crate::image_recipe::ImageRecipe, Error> {
        let input = input.into();
        if let Some(source) = self.mask_preview_source() {
            self.positive_recipe_for_source(input, roll, source, photo)
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

    pub(super) fn mask_interaction(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        layout: CanvasLayout,
        panning: bool,
        allowed: bool,
    ) {
        let (pressed, released, down, space, origin, point) = ui.input(|input| {
            (
                input.pointer.button_pressed(egui::PointerButton::Primary),
                input.pointer.button_released(egui::PointerButton::Primary),
                input.pointer.button_down(egui::PointerButton::Primary),
                input.key_down(egui::Key::Space),
                input.pointer.press_origin(),
                input.pointer.interact_pos(),
            )
        });
        if !allowed
            || !ui.is_enabled()
            || self.interaction_blocked()
            || self.fullscreen.is_some()
            || self.tab != 1
        {
            if released && self.document.visit.masks.dragging() {
                self.finish_mask(false);
            }
            return;
        }
        if !down && !released && self.document.visit.masks.dragging() {
            self.finish_mask(false);
            return;
        }
        if self
            .document
            .visit
            .masks
            .gesture()
            .is_some_and(|edit| !self.mask_is_current(edit))
        {
            self.finish_mask(false);
        }
        if pressed
            && response.is_pointer_button_down_on()
            && !space
            && !panning
            && let Some(origin) = origin.filter(|point| layout.canvas.contains(*point))
        {
            if !self.document.visit.masks.editing() && self.mask_pointer_interest(layout, origin) {
                let Some(adjustment) = self.selected_mask() else {
                    return;
                };
                let adjustment_layers::Target::Layer(id) = self.adjustment_layer_target() else {
                    return;
                };
                match adjustment.mask {
                    AdjustmentLayerCoverage::Global {} => return,
                    AdjustmentLayerCoverage::Linear { .. } => self.begin_linear_mask(Some(id)),
                    AdjustmentLayerCoverage::Radial { .. } => self.begin_radial_mask(Some(id)),
                }
            }
            self.document.visit.masks.begin_drag(layout, origin);
        }
        if !panning
            && let Some(point) = point
            && let Some(candidate) = self.document.visit.masks.update(point)
        {
            let candidate = candidate
                .and_then(|source| self.validate_source_candidate(&source).map(|_| source));
            self.document.visit.masks.preview(candidate);
            ui.ctx().request_repaint();
        }
        if response.hovered()
            && let Some(point) = point
            && self.mask_pointer_interest(layout, point)
        {
            ui.ctx().set_cursor_icon(if space {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            });
        }
        if released && self.document.visit.masks.dragging() {
            self.finish_mask(true);
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn paint_mask(&self, ui: &egui::Ui, layout: CanvasLayout) {
        if !self.adjustment_layer_overlays_visible()
            || self.fullscreen.is_some()
            || self.has_hosted_tool()
            || self.document.source
            || self.tab != 1
        {
            return;
        }
        self.document.visit.masks.paint(
            ui,
            layout,
            self.selected_mask(),
            self.document.visit.adjustment_layers.show_coverage,
        );
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

/// Contextual handles and the unfinished gesture belong to the photo visit.
#[derive(Default)]
pub(super) struct MaskEditor {
    gesture: Option<MaskEdit>,
    // Creation is a separate hosted tool; this observation does not retain it.
    creation: Weak<RefCell<MaskEdit>>,
}

impl MaskEditor {
    pub(super) fn gesture(&self) -> Option<&MaskEdit> {
        self.gesture.as_ref()
    }

    #[cfg(test)]
    pub(super) fn gesture_mut(&mut self) -> Option<&mut MaskEdit> {
        self.gesture.as_mut()
    }

    pub(super) fn editing(&self) -> bool {
        self.gesture.is_some()
    }

    pub(super) fn dragging(&self) -> bool {
        self.gesture.as_ref().is_some_and(MaskEdit::dragging)
    }

    pub(super) fn cancel(&mut self) {
        self.gesture = None;
    }

    fn begin(&mut self, edit: MaskEdit) {
        self.gesture = Some(edit);
    }

    fn finish(&mut self) -> Option<MaskEdit> {
        self.gesture.take()
    }

    fn appearance(&self, selected: Option<AdjustmentLayer>) -> Option<AdjustmentLayer> {
        let Some(edit) = &self.gesture else {
            return selected;
        };
        let id = edit.id?;
        let mut adjustment = edit
            .source
            .adjustment_layers
            .as_ref()?
            .layers
            .iter()
            .find(|record| record.id == id)?
            .adjustment;
        adjustment.mask = edit.mask?;
        Some(adjustment)
    }

    fn pointer_interest(
        &self,
        selected: Option<AdjustmentLayer>,
        layout: CanvasLayout,
        point: egui::Pos2,
    ) -> bool {
        self.editing()
            || self
                .appearance(selected)
                .is_some_and(|layer| hits(layer.mask, layout, point))
    }

    fn begin_drag(&mut self, layout: CanvasLayout, origin: egui::Pos2) {
        if let Some(edit) = &mut self.gesture {
            edit.begin_drag(layout, origin);
        }
    }

    fn update(&mut self, point: egui::Pos2) -> Option<Result<Source, Error>> {
        let edit = self.gesture.as_mut()?;
        edit.mask = Some(edit.drag.as_ref()?.update(point));
        if let Some(AdjustmentLayerCoverage::Radial { feather, .. }) = edit.mask {
            edit.feather = feather;
        }
        Some(edit.candidate().map(|(source, _)| source))
    }

    fn preview(&mut self, candidate: Result<Source, Error>) {
        if let Some(edit) = &mut self.gesture {
            match candidate {
                Ok(source) => {
                    edit.error = None;
                    edit.preview = Some(source);
                }
                Err(error) => {
                    edit.error = Some(error.to_string());
                    edit.preview = None;
                }
            }
        }
    }

    fn paint(
        &self,
        ui: &egui::Ui,
        layout: CanvasLayout,
        selected: Option<AdjustmentLayer>,
        coverage: bool,
    ) {
        if let Some(layer) = self.appearance(selected) {
            paint_geometry(
                ui,
                layout,
                layer.mask,
                layer.invert,
                coverage,
                self.dragging(),
            );
        }
    }
}

pub(super) struct CreationTool {
    edit: Rc<RefCell<MaskEdit>>,
    reveal: bool,
}

impl CreationTool {
    fn new(edit: Rc<RefCell<MaskEdit>>) -> Self {
        Self { edit, reveal: true }
    }

    fn identity(&self) -> presentation::ToolIdentity<'static> {
        match self.edit.borrow().kind {
            Kind::Linear => presentation::ToolIdentity {
                icon: icons::Icon::LinearGradient,
                title: "New linear gradient",
            },
            Kind::Radial => presentation::ToolIdentity {
                icon: icons::Icon::RadialGradient,
                title: "New radial gradient",
            },
        }
    }

    fn guidance(&self) -> &'static str {
        match self.edit.borrow().kind {
            Kind::Linear => {
                "Draw from full effect toward no effect. Release creates and selects the layer."
            }
            Kind::Radial => "Draw outward from the centre. Release creates and selects the layer.",
        }
    }

    fn finish(&mut self, editor: &mut Editor) {
        let (current, candidate) = {
            let edit = self.edit.borrow();
            (editor.mask_is_current(&edit), edit.candidate())
        };
        if !current {
            editor.status = "Mask gesture discarded: source or settings changed".into();
            editor.end_tool();
            return;
        }
        let result = candidate.and_then(|candidate| editor.accept_mask(candidate));
        match result {
            Ok(id) => {
                editor.select_adjustment_layer(adjustment_layers::Target::Layer(id));
                editor.document.visit.adjustment_layers.reveal_controls = true;
                editor.status = format!("{} selected", self.edit.borrow().kind.title());
                editor.end_tool();
            }
            Err(error) => {
                editor.status = format!("Mask rejected: {error}");
                let mut edit = self.edit.borrow_mut();
                edit.mask = None;
                edit.drag = None;
                edit.error = Some(error.to_string());
            }
        }
    }
}

impl tool::Tool for CreationTool {
    fn presentation(&self, editor: &Editor) -> tool::Presentation {
        tool::Presentation {
            identity: self.identity(),
            primary: None,
            dismiss: "Cancel",
            warning: self.edit.borrow().error.clone(),
            help: if editor.tab == 1 {
                "Draw a gradient to create a layer. Release accepts; Escape cancels.\nSpace-drag / middle-drag pans before drawing."
            } else {
                "Return to Photo to draw the gradient; Escape cancels creation."
            },
            navigation_help: None,
            fit_label: "Fit",
        }
    }

    fn drawer(&mut self, ui: &mut egui::Ui, _editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        if std::mem::take(&mut self.reveal) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        self.identity().inspector(
            ui,
            |_| (),
            |ui, ()| {
                ui.small(self.guidance());
                if let Some(error) = self.edit.borrow().error.as_deref() {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!("{error}. Draw again or cancel."),
                    );
                }
            },
        );
    }

    fn canvas(
        &mut self,
        viewer: &mut canvas::Viewer<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> Option<CanvasLayout> {
        viewer.show(
            self,
            editor,
            cx,
            Editor::photograph_controls,
            |tool, surface, editor, cx| {
                if editor.fullscreen.is_some() {
                    return surface.photograph(editor, cx);
                }
                let mut canvas = surface.selected()?;
                let (pressed, released, down, space, origin, point) = canvas.ui.input(|input| {
                    (
                        input.pointer.button_pressed(egui::PointerButton::Primary),
                        input.pointer.button_released(egui::PointerButton::Primary),
                        input.pointer.button_down(egui::PointerButton::Primary),
                        input.key_down(egui::Key::Space),
                        input.pointer.press_origin(),
                        input.pointer.interact_pos(),
                    )
                });
                let active = editor.tab == 1 && !editor.document.source;
                let allowed = active && canvas.ui.is_enabled() && !editor.interaction_blocked();
                let captured = tool.edit.borrow().dragging();
                let rect = canvas.response.rect;
                let composition = canvas.composition;
                let ppp = canvas.ui.ctx().pixels_per_point();
                let relayout =
                    |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
                let layout = canvas.layout(&mut editor.active_canvas_mut().viewport, relayout);
                let (layout, panning) = if captured {
                    (layout, false)
                } else {
                    let mut response = canvas.response.clone();
                    if active {
                        navigation::consume_primary(&mut response);
                    }
                    navigation::navigate_view(
                        &mut editor.active_canvas_mut().viewport,
                        canvas.ui,
                        &response,
                        layout,
                        relayout,
                    )
                };
                if captured && ((!allowed && released) || (!down && !released)) {
                    editor.status = "Mask gesture cancelled".into();
                    editor.end_tool();
                } else if allowed && !panning {
                    if pressed
                        && !space
                        && canvas.response.is_pointer_button_down_on()
                        && let Some(origin) = origin.filter(|point| layout.canvas.contains(*point))
                    {
                        let mut edit = tool.edit.borrow_mut();
                        edit.begin_drag(layout, origin);
                        edit.error = None;
                    }
                    {
                        let mut edit = tool.edit.borrow_mut();
                        if let (Some(drag), Some(point)) = (&edit.drag, point) {
                            edit.mask = Some(drag.update(point));
                            canvas.ui.ctx().request_repaint();
                        }
                    }
                    if released && tool.edit.borrow().dragging() {
                        tool.finish(editor);
                    }
                }
                canvas.paint_image(editor, layout, false, cx);
                if active {
                    let edit = tool.edit.borrow();
                    if let Some(mask) = edit.mask {
                        paint_geometry(
                            canvas.ui,
                            layout,
                            mask,
                            false,
                            editor.document.visit.adjustment_layers.show_coverage,
                            edit.dragging(),
                        );
                    }
                    if allowed && !panning && !space && canvas.response.hovered() {
                        canvas.ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    }
                }
                Some(layout)
            },
        )
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        if matches!(action, tool::Action::Dismiss) {
            editor.status = "Mask creation cancelled".into();
            editor.end_tool();
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
    layout: CanvasLayout,
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
            layout,
            feather,
        }
    }

    fn update(&self, point: egui::Pos2) -> AdjustmentLayerCoverage {
        let next = source_point(self.layout, point);
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
        let start = self.layout.composition.source_to_frame(self.point);
        let end = self.layout.composition.source_to_frame(next);
        let horizontal = self
            .layout
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
mod tests;
