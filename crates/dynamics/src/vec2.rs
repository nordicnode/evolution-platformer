//! Two floats and the arithmetic a solver does with them.
//!
//! A world is made of *tiles* and *ticks*: positions are in tiles, velocities in tiles per tick.
//! There is no second unit anywhere in the engine, on purpose. A tick is the sim's decision grain
//! (`sim::DT` is a sixtieth of a second) and a tile is a tile, so a gravity constant copied out of
//! `sim::physics` means the same thing here without a conversion that can be wrong.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    /// One tile right.
    pub const X: Vec2 = Vec2 { x: 1.0, y: 0.0 };
    /// One tile down. Screen and world share this direction: y grows downward, and gravity is
    /// positive, which is why nothing in the engine ever writes a minus in front of it.
    pub const Y: Vec2 = Vec2 { x: 0.0, y: 1.0 };

    #[inline]
    pub const fn new(x: f32, y: f32) -> Vec2 {
        Vec2 { x, y }
    }

    #[inline]
    pub fn dot(self, other: Vec2) -> f32 {
        self.x * other.x + self.y * other.y
    }

    #[inline]
    pub fn length_sq(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        self.length_sq().sqrt()
    }

    #[inline]
    pub fn distance(self, other: Vec2) -> f32 {
        (other - self).length()
    }

    /// The unit vector, or zero for a vector too short to have a direction.
    ///
    /// Returning zero rather than `NaN` is not politeness: two coincident particles happen in a
    /// solver (a grip placed exactly on a point, a box collapsed by a bad seed), and a `NaN` there
    /// would poison every position it touches and make the whole world untraceable. Zero is the
    /// honest answer — no direction, no correction.
    #[inline]
    pub fn normalize_or_zero(self) -> Vec2 {
        let len = self.length();
        if len > 1.0e-6 {
            self / len
        } else {
            Vec2::ZERO
        }
    }

    #[inline]
    pub fn lerp(self, other: Vec2, t: f32) -> Vec2 {
        self + (other - self) * t
    }

    /// A quarter turn clockwise, which in this y-down world points to the right of travel.
    #[inline]
    pub fn perp(self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    #[inline]
    fn add(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x + other.x, self.y + other.y)
    }
}

impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, other: Vec2) {
        self.x += other.x;
        self.y += other.y;
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    #[inline]
    fn sub(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x - other.x, self.y - other.y)
    }
}

impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, other: Vec2) {
        self.x -= other.x;
        self.y -= other.y;
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn mul(self, k: f32) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}

impl Mul<Vec2> for f32 {
    type Output = Vec2;
    #[inline]
    fn mul(self, v: Vec2) -> Vec2 {
        v * self
    }
}

impl Div<f32> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn div(self, k: f32) -> Vec2 {
        Vec2::new(self.x / k, self.y / k)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    #[inline]
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_length_vector_has_no_direction_instead_of_a_nan() {
        assert_eq!(Vec2::ZERO.normalize_or_zero(), Vec2::ZERO);
        assert_eq!(Vec2::new(3.0, 4.0).normalize_or_zero().length(), 1.0);
    }

    #[test]
    fn the_perpendicular_turns_right_in_a_y_down_world() {
        assert_eq!(Vec2::X.perp(), Vec2::new(0.0, 1.0));
    }
}
