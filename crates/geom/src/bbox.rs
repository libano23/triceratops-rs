use glam::DVec3;
use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min: DVec3,
    pub max: DVec3,
}

impl Default for BoundingBox {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl BoundingBox {
    pub const EMPTY: Self = Self {
        min: DVec3::splat(f64::INFINITY),
        max: DVec3::splat(f64::NEG_INFINITY),
    };

    pub fn new(min: DVec3, max: DVec3) -> Self {
        Self { min: min.min(max), max: min.max(max) }
    }

    pub fn from_points<'a>(pts: impl IntoIterator<Item = &'a DVec3>) -> Self {
        let mut b = Self::EMPTY;
        for p in pts {
            b.add_point(*p);
        }
        b
    }

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    pub fn add_point(&mut self, p: DVec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn union(&self, other: &Self) -> Self {
        if other.is_empty() {
            return *self;
        }
        if self.is_empty() {
            return *other;
        }
        Self { min: self.min.min(other.min), max: self.max.max(other.max) }
    }

    pub fn center(&self) -> DVec3 {
        (self.min + self.max) * 0.5
    }

    pub fn diagonal(&self) -> f64 {
        if self.is_empty() {
            0.0
        } else {
            (self.max - self.min).length()
        }
    }

    pub fn corners(&self) -> [DVec3; 8] {
        let (a, b) = (self.min, self.max);
        [
            DVec3::new(a.x, a.y, a.z),
            DVec3::new(b.x, a.y, a.z),
            DVec3::new(b.x, b.y, a.z),
            DVec3::new(a.x, b.y, a.z),
            DVec3::new(a.x, a.y, b.z),
            DVec3::new(b.x, a.y, b.z),
            DVec3::new(b.x, b.y, b.z),
            DVec3::new(a.x, b.y, b.z),
        ]
    }

    pub fn contains(&self, p: DVec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    pub fn intersects(&self, other: &BoundingBox) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.min.x <= other.max.x && other.min.x <= self.max.x && self.min.y <= other.max.y && other.min.y <= self.max.y && self.min.z <= other.max.z && other.min.z <= self.max.z
    }

    pub fn inflate(&self, d: f64) -> Self {
        Self { min: self.min - DVec3::splat(d), max: self.max + DVec3::splat(d) }
    }
}
