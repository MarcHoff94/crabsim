//! Where the 2D view looks and how close, as plain numbers. Screen points are logical pixels from
//! the canvas's top-left corner, with y down. Floor points are metres, with y up.

use crate::view::{Point, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// The floor point at the centre of the canvas.
    pub centre: Point,
    pub pixels_per_metre: f64,
}

impl Camera {
    /// About 2 km across a full-HD screen.
    pub const MIN_PIXELS_PER_METRE: f64 = 1.0;
    /// About 2 cm a pixel.
    pub const MAX_PIXELS_PER_METRE: f64 = 500.0;
    /// The least a fitted view shows each way, in metres: room for a few objects side by side.
    pub const MIN_FITTED_METRES: f64 = 20.0;
    /// Space left around the model when it is fitted into view, in pixels.
    const MARGIN: f64 = 40.0;

    /// A camera with all of `bounds` in a canvas of `size`, showing at least `MIN_FITTED_METRES`
    /// each way, so a new or small model leaves room to build on.
    pub fn fitting(bounds: Rect, size: Point) -> Camera {
        let width = (bounds.max.x - bounds.min.x).max(Self::MIN_FITTED_METRES);
        let height = (bounds.max.y - bounds.min.y).max(Self::MIN_FITTED_METRES);
        let usable = Point { x: (size.x - 2.0 * Self::MARGIN).max(1.0), y: (size.y - 2.0 * Self::MARGIN).max(1.0) };
        let pixels_per_metre = (usable.x / width).min(usable.y / height);
        Camera {
            centre: Point { x: (bounds.min.x + bounds.max.x) / 2.0, y: (bounds.min.y + bounds.max.y) / 2.0 },
            pixels_per_metre: pixels_per_metre.clamp(Self::MIN_PIXELS_PER_METRE, Self::MAX_PIXELS_PER_METRE),
        }
    }

    pub fn to_screen(&self, floor: Point, size: Point) -> Point {
        Point {
            x: size.x / 2.0 + (floor.x - self.centre.x) * self.pixels_per_metre,
            y: size.y / 2.0 - (floor.y - self.centre.y) * self.pixels_per_metre,
        }
    }

    pub fn to_world(&self, screen: Point, size: Point) -> Point {
        Point {
            x: self.centre.x + (screen.x - size.x / 2.0) / self.pixels_per_metre,
            y: self.centre.y - (screen.y - size.y / 2.0) / self.pixels_per_metre,
        }
    }

    /// Moves the floor with a pointer dragged by `delta` pixels.
    pub fn pan(&mut self, delta: Point) {
        self.centre.x -= delta.x / self.pixels_per_metre;
        self.centre.y += delta.y / self.pixels_per_metre;
    }

    /// Zooms by `factor`, more than 1 to go closer, keeping the floor point under `pointer` where
    /// it is on screen.
    pub fn zoom_at(&mut self, factor: f64, pointer: Point, size: Point) {
        let under = self.to_world(pointer, size);
        self.pixels_per_metre =
            (self.pixels_per_metre * factor).clamp(Self::MIN_PIXELS_PER_METRE, Self::MAX_PIXELS_PER_METRE);
        let now_under = self.to_world(pointer, size);
        self.centre.x += under.x - now_under.x;
        self.centre.y += under.y - now_under.y;
    }
}
