//! The world: particles, links between them, and the tick that advances both.
//!
//! # What kind of engine this is
//!
//! Position-based. A tick integrates every free body under gravity, then solver passes *project
//! positions* until the links are their length and nothing is inside the terrain, and a body's
//! velocity is recovered from how far it actually moved. There is no force accumulation and no
//! implicit solve: the correction a body receives is the smallest one that satisfies the
//! constraint, which is both cheap and easy to reason about when a rope must stay exactly as long
//! as it is.
//!
//! It is a **particle** engine, not a rigid-body one. A crate is eight particles (four corners,
//! four edges) plus two cross-braces — a rigid rectangle made of inextensible links, which can
//! tumble and rest on a corner like a solid thing because its four corners are what touch the
//! world. That is a deliberate trade: no angular velocity, no inertia tensor, no rotating collision
//! shapes, in exchange for a solver whose every step can be read. Rotation is *emergent* from the
//! corners, so a crate that hits a ledge tips over it without being told to.
//!
//! # Determinism
//!
//! Everything is `f32`, every list is a `Vec` walked in index order, and nothing consults a clock,
//! a hash map or an allocator address. Two worlds built by the same calls and stepped the same
//! number of ticks are bit-identical, which is what a replay of a physics object needs to be.
//!
//! # What it does not do
//!
//! No sleeping (a body at rest keeps being solved; drag and friction are what bring it to rest),
//! no rotation as a stored quantity, no friction cones beyond a per-tick tangential damp, and
//! restitution is applied to velocity after the contact is resolved rather than as an impulse at
//! the moment of impact. Those are all things a game of this scale does not need yet, and each is
//! a place where a bigger engine would earn its complexity.

use crate::terrain::Terrain;
use crate::vec2::Vec2;

/// How a link answers when the two bodies are the wrong distance apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// Holds the distance exactly, in both directions: a rod, a chain under tension, the edge of a
    /// crate.
    Rigid,
    /// Holds the distance as a **maximum**. Below it the link is fully slack and does nothing.
    Rope,
}

/// A constraint between two bodies.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    /// Rest length in tiles — the exact distance for [`LinkKind::Rigid`], the longest allowed one
    /// for [`LinkKind::Rope`].
    pub length: f32,
    pub kind: LinkKind,
    /// How much of its error one pass corrects, from 0 to 1. A braided rope and a steel rod are both
    /// 1; anything less is a spring with a very high rate, which exists for the future rather than
    /// for today's uses.
    pub stiffness: f32,
}

/// A body's hands on a point *along* a link.
///
/// The point is `a + (b - a) * t`, which is how a grip at an arc length on a chain is expressed
/// without adding a body to the chain: the hands hold the line between two of its particles, and
/// move with it when they are pulled. `t` is a fraction of one link, not of the whole chain.
#[derive(Clone, Copy, Debug)]
pub struct Grip {
    pub a: usize,
    pub b: usize,
    pub t: f32,
    pub target: Vec2,
}

/// One point mass.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub pos: Vec2,
    /// Tiles per tick.
    pub vel: Vec2,
    /// Zero for a pinned body. The inverse is stored because every solver term is a weighted
    /// average of inverse masses, and a pinned body is simply one that never takes a share.
    pub inv_mass: f32,
    /// Fraction of its velocity a body keeps per tick: air drag, and the only loss a free-falling
    /// line has.
    pub damping: f32,
    /// How far outside the terrain a body is held once it has been pushed out of a cell. Also the
    /// radius at which it is *tested*: a point is tested at its centre, so this is a skin, not a
    /// collision shape.
    pub skin: f32,
    /// Fraction of the tangential velocity a contact removes each tick.
    pub friction: f32,
    /// Fraction of the incoming normal velocity a contact gives back.
    pub restitution: f32,
    /// The pose at the start of the current substep, where velocity is recovered from.
    pub(crate) prev: Vec2,
    /// The velocity before the current substep's gravity, for restitution.
    pub(crate) before: Vec2,
}

impl Body {
    /// The body's mass, or infinity for a pinned body — which is the truthful reading of
    /// `inv_mass == 0` and keeps the arithmetic honest wherever mass is needed.
    pub fn mass(&self) -> f32 {
        if self.inv_mass > 0.0 {
            1.0 / self.inv_mass
        } else {
            f32::INFINITY
        }
    }
}

/// A body touched the terrain this step.
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub body: usize,
    /// The face of the cell it left by, pointing **out of the terrain**: up out of a floor, left
    /// out of the right-hand wall of a hole, and so on.
    pub normal: Vec2,
}

/// A small deterministic 2-D world.
#[derive(Clone, Debug)]
pub struct World {
    /// Tiles per tick squared, added to every free body once per tick. Positive y is down.
    pub gravity: Vec2,
    /// The most solver passes one substep may run. More passes hold a chain's length more tightly
    /// under load and cost proportionally more.
    pub iterations: usize,
    /// Stop iterating once every link is this close to satisfied, in tiles.
    pub tolerance: f32,
    /// Over-relaxation on each link correction, from 0 to 2. One is plain Gauss-Seidel.
    ///
    /// A chain's length error travels one link per pass, so at one a twenty-link line needs
    /// hundreds of passes to stop hanging a few percent long — the corrections crawl back to the
    /// tie and are still crawling when the tick ends. Pushing each correction slightly past its
    /// target is the classical fix for exactly this kind of linear system: the overshoot is what
    /// carries the error along the chain, and the system it is solving is symmetric and positive
    /// definite, which is the case over-relaxation is guaranteed to converge on. Values near two
    /// converge fastest and ring; the default is a compromise.
    pub relaxation: f32,
    /// The most a body may travel in one substep. The tick is split so that nothing moves further
    /// than this between collision tests, which is what stops a fast body passing through a
    /// one-tile wall between two frames.
    pub max_travel: f32,
    /// A cap on that split, so a body launched at absurd speed cannot make one tick cost a second.
    pub max_substeps: usize,
    bodies: Vec<Body>,
    links: Vec<Link>,
    grips: Vec<Grip>,
    contacts: Vec<Contact>,
    /// Which end the next link sweep starts from. Flipped every pass (see `solve_links`).
    sweep_backwards: bool,
}

impl World {
    /// A world whose bodies fall with `gravity` (tiles per tick squared).
    pub fn new(gravity: Vec2) -> World {
        World {
            gravity,
            iterations: 24,
            tolerance: 1.0e-3,
            relaxation: 1.6,
            max_travel: 0.5,
            max_substeps: 8,
            bodies: Vec::new(),
            links: Vec::new(),
            grips: Vec::new(),
            contacts: Vec::new(),
            sweep_backwards: false,
        }
    }

    /// Add a body at `pos`. `inv_mass` is `1 / mass`, or zero to pin it where it is.
    pub fn add_body(&mut self, pos: Vec2, inv_mass: f32) -> usize {
        self.bodies.push(Body {
            pos,
            vel: Vec2::ZERO,
            inv_mass,
            damping: 1.0,
            skin: 0.02,
            friction: 0.0,
            restitution: 0.0,
            prev: pos,
            before: Vec2::ZERO,
        });
        self.bodies.len() - 1
    }

    /// Add a body that cannot move: a tie, a hinge, a fixed pivot.
    pub fn pin(&mut self, at: Vec2) -> usize {
        self.add_body(at, 0.0)
    }

    /// A [`LinkKind::Rigid`] link of exactly `length` between two bodies.
    pub fn connect(&mut self, a: usize, b: usize, length: f32) -> usize {
        self.push_link(Link {
            a,
            b,
            length,
            kind: LinkKind::Rigid,
            stiffness: 1.0,
        })
    }

    /// A link that may be shorter than `max_length` but never longer: the line of a rope.
    pub fn tether(&mut self, a: usize, b: usize, max_length: f32) -> usize {
        self.push_link(Link {
            a,
            b,
            length: max_length,
            kind: LinkKind::Rope,
            stiffness: 1.0,
        })
    }

    fn push_link(&mut self, link: Link) -> usize {
        assert!(
            link.a < self.bodies.len() && link.b < self.bodies.len(),
            "a link must name two existing bodies"
        );
        self.links.push(link);
        self.links.len() - 1
    }

    /// Hold the point a fraction `t` along the link from `a` to `b` at `target`.
    ///
    /// Used by the grip: the hands are infinitely heavy compared with the line, and the point they
    /// hold is between two of its particles.
    pub fn grip(&mut self, a: usize, b: usize, t: f32, target: Vec2) -> usize {
        self.grips.push(Grip {
            a,
            b,
            t: t.clamp(0.0, 1.0),
            target,
        });
        self.grips.len() - 1
    }

    /// Move an existing grip to a new point and target, without disturbing the chain around it.
    pub fn set_grip(&mut self, index: usize, t: f32, target: Vec2) {
        if let Some(grip) = self.grips.get_mut(index) {
            grip.t = t.clamp(0.0, 1.0);
            grip.target = target;
        }
    }

    /// Let go of grip `index`.
    pub fn release_grip(&mut self, index: usize) {
        if index < self.grips.len() {
            self.grips.remove(index);
        }
    }

    pub fn bodies(&self) -> &[Body] {
        &self.bodies
    }

    pub fn body(&self, index: usize) -> &Body {
        &self.bodies[index]
    }

    pub fn body_mut(&mut self, index: usize) -> &mut Body {
        &mut self.bodies[index]
    }

    pub fn links(&self) -> &[Link] {
        &self.links
    }

    pub fn grips(&self) -> &[Grip] {
        &self.grips
    }

    /// Bodies that touched terrain during the last [`World::step`].
    pub fn contacts(&self) -> &[Contact] {
        &self.contacts
    }

    /// Total kinetic energy, in tiles² · mass per tick². Used by tests to say "this came to rest"
    /// without naming every body.
    pub fn energy(&self) -> f32 {
        self.bodies
            .iter()
            .map(|b| 0.5 * b.mass() * b.vel.length_sq())
            .filter(|e| e.is_finite())
            .sum()
    }

    /// The fastest body in the world, in tiles per tick.
    pub fn max_speed(&self) -> f32 {
        self.bodies
            .iter()
            .map(|b| b.vel.length())
            .fold(0.0_f32, f32::max)
    }

    /// Whether every body is slower than `speed`.
    pub fn is_resting(&self, speed: f32) -> bool {
        self.bodies.iter().all(|b| b.vel.length() <= speed)
    }

    /// A rigid rectangle of four corner bodies, joined by its four edges and two diagonals.
    ///
    /// This is the engine's crate: `mass` is spread evenly over the corners, the diagonals keep it
    /// from shearing, and its corners are what collide with the terrain — so it lands flat, rests
    /// on a ledge, and tips over an edge without any of that being written down. Returns the corner
    /// indices in the order top-left, top-right, bottom-right, bottom-left, for whoever wants to
    /// draw it.
    pub fn add_box(&mut self, min: Vec2, size: Vec2, mass: f32) -> [usize; 4] {
        let inv_mass = 4.0 / mass.max(1.0e-6);
        let corners = [
            min,
            min + Vec2::new(size.x, 0.0),
            min + Vec2::new(size.x, size.y),
            min + Vec2::new(0.0, size.y),
        ];
        let ids = corners.map(|p| self.add_body(p, inv_mass));
        for i in 0..4 {
            let next = (i + 1) % 4;
            let length = corners[i].distance(corners[next]);
            self.connect(ids[i], ids[next], length);
        }
        let diagonal = corners[0].distance(corners[2]);
        self.connect(ids[0], ids[2], diagonal);
        self.connect(ids[1], ids[3], diagonal);
        ids
    }

    /// Advance the world one tick.
    ///
    /// The tick is split into as many substeps as the fastest body needs, so a body that would
    /// move most of a tile in a tick is tested for collision several times on the way. Each
    /// substep integrates, solves and recovers velocity; the split changes nothing about the
    /// arithmetic except where the collision test happens.
    pub fn step<T: Terrain + ?Sized>(&mut self, terrain: &T) {
        self.contacts.clear();

        // Air drag is a per-tick loss, so it is applied once rather than once per substep.
        for body in &mut self.bodies {
            body.vel = body.vel * body.damping;
        }

        let fastest = self.max_speed() + self.gravity.length();
        let substeps = ((fastest / self.max_travel.max(1.0e-3)).ceil() as usize)
            .clamp(1, self.max_substeps.max(1));
        let h = 1.0 / substeps as f32;

        for _ in 0..substeps {
            for body in &mut self.bodies {
                if body.inv_mass <= 0.0 {
                    body.vel = Vec2::ZERO;
                    body.prev = body.pos;
                    body.before = Vec2::ZERO;
                    continue;
                }
                body.before = body.vel;
                body.prev = body.pos;
                body.vel += self.gravity * h;
                body.pos += body.vel * h;
            }

            for _ in 0..self.iterations {
                let mut worst = self.solve_links();
                worst = worst.max(self.solve_grips());
                worst = worst.max(self.solve_terrain(terrain));
                if worst <= self.tolerance {
                    break;
                }
            }

            self.recover(h);
        }
    }

    /// One projection pass over every link, returning the worst length error it left behind.
    fn solve_links(&mut self) -> f32 {
        let mut worst = 0.0_f32;
        // Sweeps alternate direction. Gauss-Seidel propagates a correction one link per pass, so a
        // chain solved always from the same end gathers its error at the far end — a twenty-link
        // line would hang percent-level long no matter how many passes it was given. Alternating
        // lets the correction travel back along the chain, which is what makes a modest pass count
        // hold a long line's length.
        let backwards = self.sweep_backwards;
        self.sweep_backwards = !backwards;
        for step in 0..self.links.len() {
            let i = if backwards {
                self.links.len() - 1 - step
            } else {
                step
            };
            let Link {
                a,
                b,
                length,
                kind,
                stiffness,
            } = self.links[i];
            let (wa, wb) = (self.bodies[a].inv_mass, self.bodies[b].inv_mass);
            let total = wa + wb;
            if total <= 0.0 {
                continue;
            }
            let delta = self.bodies[b].pos - self.bodies[a].pos;
            let dist = delta.length();
            if dist <= 1.0e-6 {
                continue;
            }
            let error = dist - length;
            // A rope is slack until something pulls it longer than it is.
            if kind == LinkKind::Rope && error <= 0.0 {
                continue;
            }
            worst = worst.max(error.abs());
            let normal = delta / dist;
            let correction = normal
                * (error * stiffness.clamp(0.0, 1.0) * self.relaxation.clamp(0.0, 1.99) / total);
            self.bodies[a].pos += correction * wa;
            self.bodies[b].pos -= correction * wb;
        }
        worst
    }

    /// One projection pass over every grip, returning the worst error.
    fn solve_grips(&mut self) -> f32 {
        let mut worst = 0.0_f32;
        for i in 0..self.grips.len() {
            let Grip { a, b, t, target } = self.grips[i];
            let (wa, wb) = (self.bodies[a].inv_mass, self.bodies[b].inv_mass);
            // The change at the grip per unit change of each body, weighted by the inverse masses:
            // the minimum-motion correction that puts the interpolated point on its target.
            let w0 = wa * (1.0 - t) * (1.0 - t);
            let w1 = wb * t * t;
            let total = w0 + w1;
            if total <= 0.0 {
                continue;
            }
            let held = self.bodies[a].pos.lerp(self.bodies[b].pos, t);
            let error = target - held;
            worst = worst.max(error.length());
            self.bodies[a].pos += error * (wa * (1.0 - t) / total);
            self.bodies[b].pos += error * (wb * t / total);
        }
        worst
    }

    /// Push every body out of the terrain it is inside, returning the deepest penetration found.
    ///
    /// A body leaves a cell by the face it **came in through**, plus its own skin. The entry face
    /// is not a detail. A body that arrives deep — a corner shoved half a tile into a floor by a
    /// link projection, or a fast faller caught mid-tile — is nearer the *far* side of the cell
    /// than the near one, and pushing it out that way is a solver that posts bodies through floors.
    /// The pose at the start of the substep says which side it came from, so that is the side it
    /// goes back out of, however deep it got. Only a body that was already inside when the substep
    /// began — the degenerate case, and the one a pin or an edit can produce — falls back to the
    /// nearest face.
    ///
    /// The skin is why a settled body is not re-tested inside the same cell every pass, and why a
    /// body resting on a floor sits a pixel above it, the width of a drawn cord.
    fn solve_terrain<T: Terrain + ?Sized>(&mut self, terrain: &T) -> f32 {
        let mut worst = 0.0_f32;
        for i in 0..self.bodies.len() {
            let body = &self.bodies[i];
            if body.inv_mass <= 0.0 || !body.pos.is_finite() {
                continue;
            }
            let pos = body.pos;
            let skin = body.skin.max(0.0);
            let tx = pos.x.floor() as i32;
            let ty = pos.y.floor() as i32;
            if !terrain.solid(tx, ty) {
                continue;
            }
            let to_top = pos.y - ty as f32;
            let to_bottom = ty as f32 + 1.0 - pos.y;
            let to_left = pos.x - tx as f32;
            let to_right = tx as f32 + 1.0 - pos.x;
            let prev = body.prev;
            let arrived = [
                (prev.y <= ty as f32, Vec2::new(0.0, -1.0), to_top),
                (prev.y >= ty as f32 + 1.0, Vec2::new(0.0, 1.0), to_bottom),
                (prev.x <= tx as f32, Vec2::new(-1.0, 0.0), to_left),
                (prev.x >= tx as f32 + 1.0, Vec2::new(1.0, 0.0), to_right),
            ];
            let (normal, depth) = arrived
                .iter()
                .filter(|(from, _, _)| *from)
                .map(|(_, normal, depth)| (*normal, *depth))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap_or_else(|| {
                    // Already inside when the substep began: the nearest face is the only evidence
                    // left, so take it.
                    let nearest = [
                        (Vec2::new(0.0, -1.0), to_top),
                        (Vec2::new(0.0, 1.0), to_bottom),
                        (Vec2::new(-1.0, 0.0), to_left),
                        (Vec2::new(1.0, 0.0), to_right),
                    ];
                    *nearest
                        .iter()
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .expect("four faces")
                });
            self.bodies[i].pos = pos + normal * (depth + skin);
            worst = worst.max(depth + skin);
            if !self.contacts.iter().any(|c| c.body == i) {
                self.contacts.push(Contact { body: i, normal });
            }
        }
        worst
    }

    /// Recover velocity from the distance each body actually moved, then answer the contacts.
    ///
    /// The recovery is what makes the solve a *simulation* rather than a pile of projections: a
    /// position correction that took a body backwards becomes negative velocity, so the body does
    /// not keep its momentum into the ground.
    fn recover(&mut self, h: f32) {
        for body in &mut self.bodies {
            if body.inv_mass <= 0.0 {
                body.vel = Vec2::ZERO;
            } else {
                body.vel = (body.pos - body.prev) / h;
            }
        }

        for k in 0..self.contacts.len() {
            let Contact { body, normal } = self.contacts[k];
            let b = &mut self.bodies[body];
            // Motion into the surface is spent. This is the part that position recovery alone
            // cannot do: a body that fell most of a substep before a projection caught it still has
            // that whole fall in `pos - prev`, and leaving it there would carry the body into the
            // floor again next substep — which is a body that sinks through a floor at exactly its
            // falling speed. The contact is what says how much of that travel the surface took.
            let incoming = b.before.dot(normal);
            let wanted = if b.restitution > 0.0 && incoming < -0.05 {
                // A bounce is the speed it *arrived* with, not the speed the projection left: a
                // ball that hit a floor at 0.4 and leaves at 0.2 has bounced, and the projection
                // alone would have thrown that 0.4 away.
                -b.restitution * incoming
            } else {
                0.0
            };
            let normal_speed = b.vel.dot(normal);
            if normal_speed < wanted {
                b.vel += normal * (wanted - normal_speed);
            }
            // Friction works on the slide, not on the press: the normal component is the contact's
            // business and the tangential one is the ground's.
            let normal_speed = b.vel.dot(normal);
            let tangential = b.vel - normal * normal_speed;
            b.vel = tangential * (1.0 - b.friction.clamp(0.0, 1.0)) + normal * normal_speed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pinned_body_never_moves_and_a_free_one_falls() {
        let mut world = World::new(Vec2::new(0.0, 0.05));
        let pin = world.pin(Vec2::new(1.0, 1.0));
        let free = world.add_body(Vec2::new(1.0, 2.0), 1.0);
        for _ in 0..10 {
            world.step(&crate::terrain::Open);
        }
        assert_eq!(world.body(pin).pos, Vec2::new(1.0, 1.0));
        assert!(world.body(free).pos.y > 2.0);
    }

    #[test]
    fn a_rope_link_does_not_push_its_ends_apart() {
        let mut world = World::new(Vec2::ZERO);
        let a = world.add_body(Vec2::new(0.0, 0.0), 1.0);
        let b = world.add_body(Vec2::new(0.5, 0.0), 1.0);
        world.tether(a, b, 2.0);
        world.step(&crate::terrain::Open);
        let gap = world.body(a).pos.distance(world.body(b).pos);
        assert!(
            (gap - 0.5).abs() < 1.0e-4,
            "slack rope moved its ends: {gap}"
        );
    }
}
