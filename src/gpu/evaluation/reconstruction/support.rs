//! Rectangular reads of contributing source cells, including real-edge extension.
use super::*;

impl Reconstruction {
    pub(super) fn source_parts(&self, part: Rect, output: Rect) -> Vec<Rect> {
        let corners = [
            [output.x, output.y],
            [output.right(), output.y],
            [output.right(), output.bottom()],
            [output.x, output.bottom()],
        ]
        .map(|point| point.map(f64::from));
        let polygons = [self.mapping, self.packed].map(|map| corners.map(|point| map.point(point)));
        let roundoff = if self.mapping.integral() {
            [0.; 2]
        } else {
            self.mapping.roundoff(output)
        };
        let complete = self
            .support
            .map(|support| support.coverage.corners().map(|point| point.0));
        let mut result: Vec<Rect> = Vec::new();
        for y in part.y..part.bottom() {
            let mut low = f64::INFINITY;
            let mut high = f64::NEG_INFINITY;
            for polygon in polygons {
                let mut first = f64::from(y) - roundoff[1];
                let mut end = f64::from(y) + 1. + roundoff[1];
                if self.support.is_none()
                    && let Some(bounds) = self.input.bounds
                {
                    if y == bounds.y {
                        first =
                            first.min(polygon.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min));
                    }
                    if y + 1 == bounds.bottom() {
                        end = end.max(
                            polygon
                                .iter()
                                .map(|p| p[1])
                                .fold(f64::NEG_INFINITY, f64::max),
                        );
                    }
                }
                if let Some(span) = row_span(polygon, first, end) {
                    low = low.min(span[0] - roundoff[0]);
                    high = high.max(span[1] + roundoff[0]);
                }
            }
            if !low.is_finite() || !high.is_finite() {
                continue;
            }
            if self.support.is_none()
                && let Some(bounds) = self.input.bounds
            {
                low = low.clamp(f64::from(bounds.x), f64::from(bounds.right() - 1));
                high = high.clamp(f64::from(bounds.x + 1), f64::from(bounds.right()));
            }
            if let Some(polygon) = complete {
                let Some(span) = row_span(polygon, f64::from(y), f64::from(y) + 1.) else {
                    continue;
                };
                // Complete reconstruction's existing mask admits exact source-cell coverage.
                low = low.max(span[0].floor());
                high = high.min(span[1].ceil());
            }
            let first = low.floor().max(f64::from(part.x)) as i32;
            let end = high.ceil().min(f64::from(part.right())) as i32;
            if end <= first {
                continue;
            }
            if let Some(previous) = result.last_mut()
                && previous.x == first
                && previous.right() == end
                && previous.bottom() == y
            {
                previous.height += 1;
            } else {
                result.push(Rect::new([first, y], [(end - first) as u32, 1]));
            }
        }
        result
    }
}

fn row_span(polygon: [[f64; 2]; 4], low: f64, high: f64) -> Option<[f64; 2]> {
    let mut vertices = polygon.to_vec();
    for (edge, greater) in [(low, true), (high, false)] {
        if vertices.is_empty() {
            return None;
        }
        let mut clipped = Vec::with_capacity(8);
        let inside = |point: [f64; 2]| {
            if greater {
                point[1] >= edge
            } else {
                point[1] <= edge
            }
        };
        let mut a = *vertices.last()?;
        for &b in &vertices {
            if inside(a) != inside(b) {
                let fraction = (edge - a[1]) / (b[1] - a[1]);
                clipped.push([a[0] + fraction * (b[0] - a[0]), edge]);
            }
            if inside(b) {
                clipped.push(b);
            }
            a = b;
        }
        vertices = clipped;
    }
    let min_y = vertices
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let max_y = vertices
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    if max_y <= min_y {
        return None;
    }
    Some([
        vertices
            .iter()
            .map(|point| point[0])
            .fold(f64::INFINITY, f64::min),
        vertices
            .iter()
            .map(|point| point[0])
            .fold(f64::NEG_INFINITY, f64::max),
    ])
}
