//! The pellets in the picture of the stomach, moved as the game moves them (`BiologyPanel.FixedUpdate`, `StomachContentPellet`).
//!
//! Each pellet is a `Rigidbody2D` (mass 20, linear damping 0.1, no gravity) with a circle collider of radius 10.7 x its scale and the "BouncyBox" material
//! (friction 0.4, bounciness 0.4). Every fixed step the panel pulls a pellet back inside the stomach and pushes it: 400 down above the acid, and under the
//! acid `400 * (MassDensity / 0.02 - 1)` down (plant floats up, meat sinks). Matter that is swallowed comes in from the right with a push to the left, and
//! digestion makes the pellets smaller (`ChangeAmount`: the scale follows the square root of the amount, a pellet that is used up goes).
//!
//! The engine here is small: circles against each other and against the stomach's capsule, impulses with the restitution and Coulomb friction, which
//! also spins the pellets (a disc: inertia `m r^2 / 2`). It runs on the real
//! time that passes between two drawn frames, and stands still while the game is paused. In the tests nothing moves.

use std::cell::RefCell;
use std::time::Instant;

use bibites_core::organs::StomachMaterial;

use crate::internals::{self, Pellet, StomachInfo, StomachShape};

/// The step the game's physics runs at (`Time.fixedDeltaTime` at speed 1).
const STEP: f32 = 0.02;
/// `Rigidbody2D.mass`.
const MASS: f32 = 20.0;
/// `Rigidbody2D.linearDamping`.
const DAMPING: f32 = 0.1;
/// `PhysicsMaterial2D.bounciness` of "BouncyBox".
const BOUNCE: f32 = 0.4;
/// `PhysicsMaterial2D.friction` of "BouncyBox".
const FRICTION: f32 = 0.4;
/// `Rigidbody2D.angularDamping` (Unity's default).
const ANGULAR_DAMPING: f32 = 0.05;
/// The picture is 305 wide (`StomachContentPellet` entry speed is a tenth of it per second).
const PICTURE_WIDTH: f32 = 305.0;
/// The middle row of the picture.
const MIDDLE: f32 = 97.5;

#[derive(Clone, Copy, Debug)]
struct Body {
    pos: [f32; 2],
    vel: [f32; 2],
    /// Turn in radians and its speed (positive turns clockwise on the screen, y down).
    angle: f32,
    spin: f32,
    amount: f32,
    scale: f32,
    material: StomachMaterial,
}

struct Sim {
    owner: u64,
    shape: StomachShape,
    bodies: Vec<Body>,
    last: Instant,
    /// Time not yet run (the fixed steps).
    owed: f32,
    rng: u64,
}

thread_local! {
    static SIM: RefCell<Option<Sim>> = const { RefCell::new(None) };
}

impl Sim {
    fn new(owner: u64, info: &StomachInfo, shape: &StomachShape) -> Self {
        let bodies = internals::pellets(info, shape, owner)
            .into_iter()
            .map(|p| Body {
                pos: p.pos,
                vel: [0.0, 0.0],
                angle: p.rotation,
                spin: 0.0,
                amount: p.amount,
                scale: p.size / 20.0,
                material: p.material,
            })
            .collect();
        Self {
            owner,
            shape: *shape,
            bodies,
            last: Instant::now(),
            owed: 0.0,
            rng: owner ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn random(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        ((self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
    }

    /// `BiologyPanel.OnMatterSwallow` / `OnMatterVomit` / `AnimateStomach`, from what the stomach holds now.
    fn reconcile(&mut self, info: &StomachInfo, dt: f32) {
        let (cx, r, _, _) = self.shape.pixels();
        let d = 5.0 * (self.shape.w - 2.0 * self.shape.r);
        for item in &info.items {
            // Digestion: the pellets of the material shrink by the amount digested, shared out.
            let mut digested = dt * item.digestion_rate;
            let count = self.bodies.iter().filter(|b| b.material == item.material).count();
            if count > 0 && digested > 0.0 {
                let share = digested / count as f32;
                for body in self.bodies.iter_mut().filter(|b| b.material == item.material) {
                    let before = body.amount;
                    body.amount -= share;
                    if body.amount > 0.0 {
                        body.scale *= (body.amount / before).sqrt();
                    }
                    digested -= share.min(before);
                }
                let _ = digested;
            }
            self.bodies
                .retain(|b| b.material != item.material || b.amount > 0.0);
            // What the pellets hold against what the stomach holds.
            let held: f32 = self
                .bodies
                .iter()
                .filter(|b| b.material == item.material)
                .map(|b| b.amount)
                .sum();
            let threshold = info.area / 20.0;
            if item.amount - held > threshold || (held <= 0.0 && item.amount > 0.0) {
                // Swallowed: a pellet enters from the right and heads left.
                let amount = item.amount - held;
                let scale = 0.8 * ((amount / info.area).sqrt() * r / 10.0).max(r / 100.0);
                let kick = (self.random() - 0.5) * 0.2;
                self.bodies.push(Body {
                    pos: [cx + d / 2.0 + r / 1.1, MIDDLE],
                    vel: [-PICTURE_WIDTH / 10.0, kick * PICTURE_WIDTH / 10.0],
                    angle: 0.0,
                    spin: 0.0,
                    amount,
                    scale,
                    material: item.material,
                });
            } else if held - item.amount > 1e-3 && held > 0.0 {
                // Vomited (or a rounding): the pellets give back what is missing.
                let factor = (item.amount / held).max(0.0);
                for body in self.bodies.iter_mut().filter(|b| b.material == item.material) {
                    body.amount *= factor;
                    body.scale *= factor.sqrt();
                }
            }
        }
        // A material that left the stomach takes its pellets with it.
        let present: Vec<StomachMaterial> = info.items.iter().map(|i| i.material).collect();
        self.bodies.retain(|b| present.contains(&b.material));
    }

    /// One fixed step.
    fn step(&mut self, acid: f32) {
        let (cx, r, _, _) = self.shape.pixels();
        let d = 5.0 * (self.shape.w - 2.0 * self.shape.r);
        let half_d = d / 2.0;
        // Forces: 400 down, or `400 * (MassDensity / 0.02 - 1)` down under the acid (y grows downwards here).
        for body in &mut self.bodies {
            // The height inside the stomach, from the bottom (`InverseLerp(-r, r, y)` with y up).
            let from_bottom = (MIDDLE + r - body.pos[1]) / (2.0 * r).max(1e-6);
            let force = if from_bottom <= acid {
                400.0 * (body.material.params().mass_density / 0.02 - 1.0)
            } else {
                400.0
            };
            body.vel[1] += force / MASS * STEP;
            let damp = 1.0 / (1.0 + DAMPING * STEP);
            body.vel[0] *= damp;
            body.vel[1] *= damp;
            body.pos[0] += body.vel[0] * STEP;
            body.pos[1] += body.vel[1] * STEP;
            body.spin *= 1.0 / (1.0 + ANGULAR_DAMPING * STEP);
            body.angle += body.spin * STEP;
        }
        // The stomach's walls: `FixedUpdate` pulls a pellet back to `r - radius`, and the edge collider bounces it.
        for body in &mut self.bodies {
            let radius = 10.0 * body.scale;
            let limit = (r - radius).max(0.0);
            let local = [body.pos[0] - cx, body.pos[1] - MIDDLE];
            let (nx, ny, dist) = if local[0].abs() < half_d {
                (0.0, local[1].signum(), local[1].abs())
            } else {
                let from_end = [local[0] - local[0].signum() * half_d, local[1]];
                let m = (from_end[0] * from_end[0] + from_end[1] * from_end[1]).sqrt().max(1e-6);
                (from_end[0] / m, from_end[1] / m, m)
            };
            if dist > limit {
                let over = dist - limit;
                body.pos[0] -= nx * over;
                body.pos[1] -= ny * over;
                let along = body.vel[0] * nx + body.vel[1] * ny;
                if along > 0.0 {
                    body.vel[0] -= (1.0 + BOUNCE) * along * nx;
                    body.vel[1] -= (1.0 + BOUNCE) * along * ny;
                    // Friction along the wall: the contact point moves with `v + spin x radius`; a disc takes `jt = -vt m / 3`, at most `friction * jn`.
                    let (tx, ty) = (-ny, nx);
                    let slip = body.vel[0] * tx + body.vel[1] * ty + body.spin * radius;
                    let limit = FRICTION * (1.0 + BOUNCE) * along;
                    let jt = (-slip / 3.0).clamp(-limit, limit);
                    body.vel[0] += jt * tx;
                    body.vel[1] += jt * ty;
                    body.spin += 2.0 * jt / radius.max(1e-3);
                }
            }
        }
        // Pellets against pellets (circle colliders of 10.7 x scale, equal masses).
        let n = self.bodies.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let (a, b) = (self.bodies[i], self.bodies[j]);
                let (dx, dy) = (b.pos[0] - a.pos[0], b.pos[1] - a.pos[1]);
                let reach = 10.7 * (a.scale + b.scale);
                let dist2 = dx * dx + dy * dy;
                if dist2 >= reach * reach || dist2 <= 1e-9 {
                    continue;
                }
                let dist = dist2.sqrt();
                let (nx, ny) = (dx / dist, dy / dist);
                let push = (reach - dist) / 2.0;
                self.bodies[i].pos[0] -= nx * push;
                self.bodies[i].pos[1] -= ny * push;
                self.bodies[j].pos[0] += nx * push;
                self.bodies[j].pos[1] += ny * push;
                let approach = (b.vel[0] - a.vel[0]) * nx + (b.vel[1] - a.vel[1]) * ny;
                if approach < 0.0 {
                    let impulse = -(1.0 + BOUNCE) * approach / 2.0;
                    self.bodies[i].vel[0] -= impulse * nx;
                    self.bodies[i].vel[1] -= impulse * ny;
                    self.bodies[j].vel[0] += impulse * nx;
                    self.bodies[j].vel[1] += impulse * ny;
                    // Friction between the two discs (contact points at `+n ra` on a and `-n rb` on b).
                    let (tx, ty) = (-ny, nx);
                    let (ra, rb) = (10.0 * a.scale, 10.0 * b.scale);
                    let (va, vb) = (self.bodies[i], self.bodies[j]);
                    let slip = (vb.vel[0] - va.vel[0]) * tx + (vb.vel[1] - va.vel[1]) * ty - vb.spin * rb - va.spin * ra;
                    let limit = FRICTION * impulse;
                    let jt = (-slip / 6.0).clamp(-limit, limit);
                    self.bodies[i].vel[0] -= jt * tx;
                    self.bodies[i].vel[1] -= jt * ty;
                    self.bodies[j].vel[0] += jt * tx;
                    self.bodies[j].vel[1] += jt * ty;
                    self.bodies[i].spin -= 2.0 * jt / ra.max(1e-3);
                    self.bodies[j].spin -= 2.0 * jt / rb.max(1e-3);
                }
            }
        }
    }
}

/// The pellets to draw now. `owner` stands for the selected bibite (a new one starts a new stomach), `running` is false while the game is paused.
pub fn pellets_now(owner: u64, info: &StomachInfo, shape: &StomachShape, running: bool) -> Vec<Pellet> {
    SIM.with(|cell| {
        let mut cell = cell.borrow_mut();
        let fresh = match cell.as_ref() {
            Some(sim) => sim.owner != owner || sim.shape != *shape,
            None => true,
        };
        if fresh {
            *cell = Some(Sim::new(owner, info, shape));
        }
        let sim = cell.as_mut().expect("the simulation");
        let now = Instant::now();
        let elapsed = now.duration_since(sim.last).as_secs_f32().min(0.1);
        sim.last = now;
        if running && !cfg!(test) {
            sim.reconcile(info, elapsed);
            sim.owed += elapsed;
            let mut steps = 0;
            while sim.owed >= STEP && steps < 5 {
                sim.step(info.acid);
                sim.owed -= STEP;
                steps += 1;
            }
            sim.owed = sim.owed.min(STEP);
        }
        sim.bodies
            .iter()
            .map(|b| Pellet {
                pos: b.pos,
                size: 20.0 * b.scale,
                material: b.material,
                amount: b.amount,
                rotation: b.angle,
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(plant: f32, meat: f32) -> StomachInfo {
        let mut items = Vec::new();
        for (material, amount) in [(StomachMaterial::Plant, plant), (StomachMaterial::Meat, meat)] {
            if amount > 0.0 {
                items.push(internals::StomachItem {
                    material,
                    amount,
                    average_chunk: 0.0,
                    digestion_rate: 0.5,
                    efficiency: 0.5,
                });
            }
        }
        StomachInfo {
            area: 60.0,
            total: plant + meat,
            acid: 0.5,
            bonus: 0.0,
            malus: 0.0,
            effectiveness: 1.0,
            portion: 0.2,
            items,
        }
    }

    fn shape() -> StomachShape {
        StomachShape {
            offset: [32.0, 19.5],
            r: 13.5,
            w: 26.0,
            womb: [60.0, 20.0],
        }
    }

    #[test]
    fn pellets_fall_bounce_and_stay_inside_the_stomach() {
        let stomach = info(20.0, 10.0);
        let mut sim = Sim::new(5, &stomach, &shape());
        let before: f32 = sim.bodies.iter().map(|b| b.pos[1]).sum::<f32>() / sim.bodies.len() as f32;
        for _ in 0..400 {
            sim.step(0.0);
        }
        let after: f32 = sim.bodies.iter().map(|b| b.pos[1]).sum::<f32>() / sim.bodies.len() as f32;
        assert!(after > before, "no acid: they fall to the bottom ({before} -> {after})");
        let (cx, r, _, _) = shape().pixels();
        let d = 5.0 * (shape().w - 2.0 * shape().r);
        for body in &sim.bodies {
            let local = [body.pos[0] - cx, body.pos[1] - MIDDLE];
            let inside = if local[0].abs() < d / 2.0 {
                local[1].abs()
            } else {
                let e = [local[0] - local[0].signum() * d / 2.0, local[1]];
                (e[0] * e[0] + e[1] * e[1]).sqrt()
            };
            assert!(inside <= r + 0.01, "{body:?} is out of the stomach ({inside} > {r})");
        }
    }

    #[test]
    fn friction_against_the_wall_and_each_other_spins_the_pellets() {
        let stomach = info(20.0, 10.0);
        let mut sim = Sim::new(5, &stomach, &shape());
        for _ in 0..300 {
            sim.step(0.0);
        }
        assert!(sim.bodies.iter().any(|b| b.spin.abs() > 1e-3 || (b.angle - 0.0).abs() > 1e-2), "something turns");
        assert!(sim.bodies.iter().all(|b| b.spin.is_finite() && b.angle.is_finite()));
    }

    #[test]
    fn plant_floats_and_meat_sinks_in_full_acid() {
        let stomach = info(15.0, 15.0);
        let mut sim = Sim::new(9, &stomach, &shape());
        for _ in 0..1500 {
            sim.step(1.0);
        }
        let mean = |material: StomachMaterial| {
            let all: Vec<f32> = sim.bodies.iter().filter(|b| b.material == material).map(|b| b.pos[1]).collect();
            all.iter().sum::<f32>() / all.len() as f32
        };
        assert!(mean(StomachMaterial::Plant) < mean(StomachMaterial::Meat), "plant is above the meat");
    }

    #[test]
    fn digestion_shrinks_the_pellets_and_a_swallow_brings_one_in() {
        let mut stomach = info(20.0, 0.0);
        let mut sim = Sim::new(3, &stomach, &shape());
        let held = |sim: &Sim| sim.bodies.iter().map(|b| b.amount).sum::<f32>();
        let start = held(&sim);
        assert!((start - 20.0).abs() < 1e-3);
        stomach.items[0].amount = 19.0;
        sim.reconcile(&stomach, 1.0);
        assert!(held(&sim) < start, "digestion made them smaller");
        let count = sim.bodies.len();
        // 12 more units arrive (more than a twentieth of the stomach): one pellet comes in from the right, heading left.
        stomach.items[0].amount = held(&sim) + 12.0;
        sim.reconcile(&stomach, 0.0);
        assert_eq!(sim.bodies.len(), count + 1);
        let new = sim.bodies.last().expect("the new pellet");
        assert!(new.vel[0] < 0.0 && (new.amount - 12.0).abs() < 1e-3);
    }
}
