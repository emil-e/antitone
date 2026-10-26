use super::*;
use crate::processing::InputSettings;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
    geometry::SourcePoint,
    project::AdjustmentLayerId,
    viewport::CanvasLayout,
};
use glam::DVec2;
#[path = "editor_radial_masks.rs"]
mod radial;

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
                "Draw toward the affected side. Drag endpoints to change the fade; drag the connector to move it. Release accepts; Escape cancels this gesture."
            }
            Self::Radial => {
                "Draw outward from the centre. Drag inside to move, squares to reshape, the outer handle to rotate, or the inner diamond to feather. Release accepts; Escape cancels this gesture."
            }
        }
    }
}

enum MaskDrag {
    Linear(LinearDrag),
    Radial(radial::RadialDrag),
}

impl MaskDrag {
    fn update(&self, point: egui::Pos2) -> AdjustmentLayerCoverage {
        match self {
            Self::Linear(drag) => drag.update(point),
            Self::Radial(drag) => drag.update(point),
        }
    }
}

/// Only an armed creation or a captured pointer gesture, never a persistent editing mode.
pub(super) struct MaskEdit {
    loaded: u64,
    source: Source,
    roll: RollSettings,
    input: InputSettings,
    id: Option<AdjustmentLayerId>,
    above: Option<AdjustmentLayerId>,
    mask: Option<AdjustmentLayerCoverage>,
    preview: Option<Source>,
    compare: bool,
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

    pub(super) fn creation_label(&self) -> String {
        format!(
            "{} {}",
            if self.creating() { "Draw" } else { "Refining" },
            self.kind.title().to_lowercase()
        )
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
                start: self.point,
                end: next,
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
        AdjustmentLayerCoverage::Radial { .. } => radial::hit(mask, layout, point),
    }
}

impl Editor {
    fn mask_source(&self) -> Result<Source, Error> {
        let mut source = self
            .document
            .project
            .source(
                self.current_source
                    .ok_or(Error::InvalidParameter("no selected source"))?,
            )?
            .clone();
        self.overlay_adjustment_layer_settings(&mut source, self.adjustment_layer_settings);
        source.orientation = self.photo_orientation;
        source.crop = self.crop;
        Ok(source)
    }

    fn selected_mask(&self) -> Option<AdjustmentLayer> {
        let adjustment_layers::Target::Layer(id) = self.adjustment_layer_target() else {
            return None;
        };
        self.document
            .project
            .source(self.current_source?)
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
        if !(self.tab == 1) {
            return None;
        }
        self.document
            .visit
            .mask_edit
            .as_ref()
            .map(|edit| edit.kind)
            .or_else(|| {
                self.selected_mask()
                    .map(|adjustment| match adjustment.mask {
                        AdjustmentLayerCoverage::Global {} => {
                            unreachable!("global layers have no mask handles")
                        }
                        AdjustmentLayerCoverage::Linear { .. } => Kind::Linear,
                        AdjustmentLayerCoverage::Radial { .. } => Kind::Radial,
                    })
            })
            .map(Kind::help)
    }

    pub(super) fn mask_handles_available(&self) -> bool {
        self.adjustment_layer_overlays_visible()
            && !self.document.visit.adjustment_layers.dragging()
            && self.tab == 1
            && !self.busy
            && self.document.visit.ready.is_some()
            && self.input.ready()
            && !self.source
            && self.pick.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.auto_crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.document.analysis_review.is_none()
            && !self.document.dust.cleanup
            && !self.document.visit.adjustment_layers.confirming()
            && self.fullscreen.is_none()
    }

    pub(super) fn mask_pointer_interest(&self, layout: CanvasLayout, point: egui::Pos2) -> bool {
        self.mask_handles_available()
            && (self.document.visit.mask_edit.is_some()
                || self
                    .selected_mask()
                    .is_some_and(|adjustment| hits(adjustment.mask, layout, point)))
    }

    pub(super) fn begin_linear_mask(&mut self, id: Option<AdjustmentLayerId>) {
        self.begin_mask(Kind::Linear, id);
    }

    pub(super) fn begin_radial_mask(&mut self, id: Option<AdjustmentLayerId>) {
        self.begin_mask(Kind::Radial, id);
    }

    fn begin_mask(&mut self, kind: Kind, id: Option<AdjustmentLayerId>) {
        if self.busy
            || self.document.visit.ready.is_none()
            || self.source
            || self.pick.is_some()
            || self.document.visit.crop_edit.is_some()
            || self.document.auto_crop_edit.is_some()
            || self.flat_edit.is_some()
            || self.document.visit.mask_edit.is_some()
            || self.document.visit.adjustment_layers.confirming()
            || self.document.analysis_review.is_some()
            || self.fullscreen.is_some()
        {
            return;
        }
        self.finish_dust_cleanup();
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
                self.document.visit.mask_edit = Some(MaskEdit {
                    loaded: self.id,
                    source,
                    roll: self.roll,
                    input: self.input,
                    id,
                    above: match self.adjustment_layer_target() {
                        adjustment_layers::Target::Layer(id) => Some(id),
                        adjustment_layers::Target::None => None,
                    },
                    mask,
                    preview: None,
                    compare: self.working.compare,
                    kind,
                    feather: match mask {
                        Some(AdjustmentLayerCoverage::Radial { feather, .. }) => feather,
                        _ => controls::DEFAULT_RADIAL_FEATHER,
                    },
                    drag: None,
                    error: None,
                });
                self.working.compare = false;
                self.status = format!("Draw {} · Escape cancels", kind.title().to_lowercase());
            }
            Err(error) => self.status = format!("Mask unavailable: {error}"),
        }
    }

    fn mask_is_current(&self, edit: &MaskEdit) -> bool {
        edit.loaded == self.id
            && self.mask_source().ok().as_ref() == Some(&edit.source)
            && self.roll == edit.roll
            && self.input == edit.input
    }

    pub(super) fn finish_mask(&mut self, accept: bool) {
        let Some(mut edit) = self.document.visit.mask_edit.take() else {
            return;
        };
        self.working.compare = edit.compare;
        if !accept || !(self.tab == 1) {
            self.status = "Mask gesture cancelled".into();
            return;
        }
        if !self.mask_is_current(&edit) {
            self.status = "Mask gesture discarded: source or settings changed".into();
            return;
        }
        let result = edit.candidate().and_then(|(candidate, id)| {
            self.validate_source_candidate(&candidate)?;
            self.edit_adjustment_layers(|layers| {
                *layers = candidate
                    .adjustment_layers
                    .expect("candidate has local records");
                Ok(())
            })?;
            Ok(id)
        });
        match result {
            Ok(id) => {
                let created = edit.creating();
                self.select_adjustment_layer(adjustment_layers::Target::Layer(id));
                if created {
                    self.document.visit.adjustment_layers.reveal_controls = true;
                }
                self.status = format!("{} selected", edit.kind.title());
            }
            Err(error) => {
                self.status = format!("Mask rejected: {error}");
                if edit.creating() {
                    edit.mask = None;
                    edit.preview = None;
                    edit.drag = None;
                    edit.error = Some(error.to_string());
                    self.working.compare = false;
                    self.document.visit.mask_edit = Some(edit);
                }
            }
        }
    }

    pub(super) fn mask_preview_source(&self) -> Option<&Source> {
        if self.fullscreen.is_some() || self.tab != 1 || !(self.tab == 1) {
            return None;
        }
        self.document
            .visit
            .mask_edit
            .as_ref()
            .filter(|edit| edit.id.is_some() && self.mask_is_current(edit))
            .and_then(|edit| edit.preview.as_ref())
    }

    pub(super) fn view_processing(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: AdjustmentLayerSettings,
    ) -> Result<ProcessingInput, Error> {
        let input = input.into();
        if let Some(source) = self.mask_preview_source() {
            self.positive_processing_for_source(input, roll, source, photo)
        } else {
            self.positive_processing(input, roll, photo)
        }
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
        if !allowed || !ui.is_enabled() || self.busy || self.fullscreen.is_some() || self.tab != 1 {
            if released
                && self
                    .document
                    .visit
                    .mask_edit
                    .as_ref()
                    .is_some_and(MaskEdit::dragging)
            {
                self.finish_mask(false);
            }
            return;
        }
        if !down
            && !released
            && self
                .document
                .visit
                .mask_edit
                .as_ref()
                .is_some_and(MaskEdit::dragging)
        {
            self.finish_mask(false);
            return;
        }
        if pressed
            && response.is_pointer_button_down_on()
            && !space
            && !panning
            && let Some(origin) = origin.filter(|point| layout.canvas.contains(*point))
        {
            if self.document.visit.mask_edit.is_none() && self.mask_pointer_interest(layout, origin)
            {
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
            if let Some(edit) = &mut self.document.visit.mask_edit {
                edit.drag = Some(match edit.kind {
                    Kind::Linear => MaskDrag::Linear(LinearDrag {
                        handle: if edit.creating() {
                            Handle::New
                        } else {
                            handle_at(edit.mask, layout, origin)
                        },
                        point: source_point(layout, origin),
                        mask: edit.mask,
                        layout,
                    }),
                    Kind::Radial => MaskDrag::Radial(radial::RadialDrag::begin(
                        if edit.creating() { None } else { edit.mask },
                        layout,
                        origin,
                        edit.feather,
                    )),
                });
            }
        }
        if !panning
            && let Some(point) = point
            && let Some(mut edit) = self.document.visit.mask_edit.take()
        {
            if let Some(drag) = &edit.drag {
                edit.mask = Some(drag.update(point));
                if let Some(AdjustmentLayerCoverage::Radial { feather, .. }) = edit.mask {
                    edit.feather = feather;
                }
                edit.preview = match edit
                    .candidate()
                    .and_then(|(source, _)| self.validate_source_candidate(&source).map(|_| source))
                {
                    Ok(source) => {
                        edit.error = None;
                        Some(source)
                    }
                    Err(error) => {
                        edit.error = Some(error.to_string());
                        None
                    }
                };
                ui.ctx().request_repaint();
            }
            self.document.visit.mask_edit = Some(edit);
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
        if released
            && self
                .document
                .visit
                .mask_edit
                .as_ref()
                .is_some_and(MaskEdit::dragging)
        {
            self.finish_mask(true);
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn paint_mask(&self, ui: &egui::Ui, layout: CanvasLayout) {
        if !self.adjustment_layer_overlays_visible()
            || self.fullscreen.is_some()
            || self.document.analysis_review.is_some()
            || self.pick.is_some()
            || self.document.visit.crop_edit.is_some()
            || self.source
            || self.tab != 1
            || self.document.dust.cleanup
        {
            return;
        }
        let selected = self.selected_mask();
        let mask = match &self.document.visit.mask_edit {
            Some(edit) => edit.mask,
            None => selected.map(|adjustment| adjustment.mask),
        };
        let Some(mask) = mask else {
            return;
        };
        let invert = selected.is_some_and(|adjustment| adjustment.invert)
            && !self
                .document
                .visit
                .mask_edit
                .as_ref()
                .is_some_and(MaskEdit::creating);
        let painter = ui.painter().with_clip_rect(layout.canvas);
        if self.document.visit.adjustment_layers.show_coverage {
            paint_coverage(
                &painter,
                layout,
                mask,
                invert,
                ui.visuals().selection.stroke.color,
            );
        }
        if let AdjustmentLayerCoverage::Radial { .. } = mask {
            radial::paint(&painter, layout, mask);
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
            if self
                .document
                .visit
                .mask_edit
                .as_ref()
                .is_some_and(MaskEdit::dragging)
                || hover.is_some_and(|hover| hover.distance(point) < 15.)
            {
                let text = if full { "100%" } else { "0%" };
                let galley = painter.layout_no_wrap(
                    text.into(),
                    egui::FontId::proportional(12.),
                    egui::Color32::WHITE,
                );
                let rect = egui::Rect::from_min_size(point + egui::vec2(9., -18.), galley.size())
                    .expand(3.);
                painter.rect_filled(rect, 2., egui::Color32::BLACK);
                painter.galley(rect.min + egui::vec2(3., 3.), galley, egui::Color32::WHITE);
            }
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

#[cfg(test)]
#[path = "editor_masks_tests.rs"]
mod tests;
