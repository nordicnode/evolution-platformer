//! What the engine is allowed to know about the ground.
//!
//! One question, `solid(tx, ty)`, and nothing else: a level, an editor's scratch map and a test
//! grid all answer it, and the engine never needs to know which one it is holding. Keeping the
//! question this narrow is what lets the same solver run against `sim::Level` without `dynamics`
//! depending on `sim` — the arrow between the crates points one way only, so a simulated object
//! can use the engine without a cycle.

/// A world of whole-tile cells, each either empty or solid.
pub trait Terrain {
    /// Whether the cell at tile column `tx`, row `ty` is solid.
    ///
    /// Outside the map must read as **solid**. A body that walks off the edge of a level should
    /// fall, but a *grid* has no edge to fall from, and a rope that is pushed out of the world can
    /// never be pulled back in. Making the void solid is the conservative answer, and the one that
    /// keeps a solver's arithmetic inside the map.
    fn solid(&self, tx: i32, ty: i32) -> bool;
}

/// No terrain at all: a world that can fall forever.
///
/// The pendulum and the determinism tests use it, and so does anything that wants the engine
/// without a level — the point of a separate engine is that it can be run on its own.
pub struct Open;

impl Terrain for Open {
    fn solid(&self, _tx: i32, _ty: i32) -> bool {
        false
    }
}

/// A rectangular grid of solid and empty cells, in row-major order.
///
/// This is the engine's own terrain, for tests and for the box demo: the same collision code the
/// game runs, against a map that is a `Vec<bool>` rather than a level file.
#[derive(Clone, Debug)]
pub struct Grid {
    width: i32,
    height: i32,
    solid: Vec<bool>,
}

impl Grid {
    /// A `width` by `height` map with every cell in the state `solid`.
    pub fn new(width: i32, height: i32, solid: bool) -> Grid {
        Grid {
            width,
            height,
            solid: vec![solid; (width.max(0) * height.max(0)) as usize],
        }
    }

    /// A map from rows of text: `#` is solid, anything else is empty.
    ///
    /// Written for tests, where a terrain should be readable at the call site rather than
    /// assembled from a dozen `set` calls. Rows are padded to the widest one.
    pub fn from_rows(rows: &[&str]) -> Grid {
        let width = rows.iter().map(|r| r.len()).max().unwrap_or(0) as i32;
        let height = rows.len() as i32;
        let mut grid = Grid::new(width, height, false);
        for (y, row) in rows.iter().enumerate() {
            for (x, ch) in row.chars().enumerate() {
                grid.set(x as i32, y as i32, ch == '#');
            }
        }
        grid
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// Set one cell, silently ignoring coordinates outside the map.
    pub fn set(&mut self, tx: i32, ty: i32, solid: bool) {
        if tx < 0 || ty < 0 || tx >= self.width || ty >= self.height {
            return;
        }
        let i = (ty * self.width + tx) as usize;
        self.solid[i] = solid;
    }

    /// Fill the row `ty` from column `x0` to `x1` inclusive: the floor and ledge of a test level.
    pub fn fill_row(&mut self, ty: i32, x0: i32, x1: i32, solid: bool) {
        for tx in x0..=x1 {
            self.set(tx, ty, solid);
        }
    }
}

impl Terrain for Grid {
    fn solid(&self, tx: i32, ty: i32) -> bool {
        if tx < 0 || ty < 0 || tx >= self.width || ty >= self.height {
            // The void outside a grid is solid, as the trait asks. A test that wants a pit
            // writes one into the map.
            return true;
        }
        self.solid[(ty * self.width + tx) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_reads_its_rows_and_the_void_outside_is_solid() {
        let grid = Grid::from_rows(&["...", "###"]);
        assert!(!grid.solid(0, 0));
        assert!(grid.solid(0, 1));
        assert!(grid.solid(-1, 0));
        assert!(grid.solid(0, 2));
    }
}
