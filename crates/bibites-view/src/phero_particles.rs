//! The pheromone particles of the game (`PheromoneSpot.FixedUpdate`, `ParticlesMaster`, the three `ParticleSystem`s `Red`, `Green` and `Blue`).
//!
//! A spot does not draw itself: every 0.25 s of game time it adds `1.25 * pheromonesStrength` to a counter for each colour whose strength is still above zero
//! and tells the particle master to emit the whole part of the counter at its place (when "Show Pheromones" is on); a new spot starts each counter at
//! `sqrt(strength) * Random(0.5, 1.5)`. The particle systems are cones (angle 25 degrees, base radius 1) that point along the camera's axis, so what shows
//! is the sideways part of the speed; the particle is a quad mesh without a texture (a flat square) turned 45 degrees, `startSize` big, which lives
//! 2 to 3 seconds, fades in over the first tenth and out over the last fifth of it and changes colour along a two-colour gradient (the blue one also
//! shrinks to nothing). All numbers are read from the prefab (`ParticlesSpawner` of level2).
//!
//! The state is only for the eye: it lives here, advances with the game's own clock (so a pause stops it) and is not saved.

use std::collections::HashMap;
use std::cell::RefCell;

use bibites_core::ids::PheromoneId;
use bibites_core::math::Vec2;
use bibites_core::world::World;

/// `PheromoneSpot.DissipationPeriod`.
const PERIOD: f32 = 0.25;
/// `PheromoneSpot` adds this times the strength setting to a counter each period.
const COUNT_PER_PERIOD: f32 = 1.25;
/// A cap on what is alive at once (the systems' `maxParticles` is 100 000; the picture does not need that many).
const MAX_PARTICLES: usize = 30_000;

/// One colour's particle system.
struct System {
    life: (f32, f32),
    speed: (f32, f32),
    size: (f32, f32),
    /// The colour at the start and at the end of the life.
    colors: [[f32; 3]; 2],
    /// Alpha keys (time as a fraction of the life, alpha).
    alpha: [(f32, f32); 4],
    /// Whether the size shrinks to nothing over the life.
    shrinks: bool,
}

const SYSTEMS: [System; 3] = [
    System {
        life: (2.0, 3.0),
        speed: (7.0, 12.0),
        size: (1.0, 2.0),
        colors: [[1.0, 0.0, 0.0], [1.0, 0.597, 0.0]],
        alpha: [(0.0, 0.0), (0.079, 1.0), (0.815, 1.0), (1.0, 0.0)],
        shrinks: false,
    },
    System {
        life: (2.0, 3.0),
        speed: (15.0, 25.0),
        size: (0.5, 1.5),
        colors: [[0.024, 1.0, 0.051], [0.78, 1.0, 0.024]],
        alpha: [(0.0, 0.0), (0.071, 1.0), (0.826, 1.0), (1.0, 0.0)],
        shrinks: false,
    },
    System {
        life: (2.0, 3.0),
        speed: (10.0, 15.0),
        size: (1.0, 3.0),
        colors: [[0.0, 0.225, 1.0], [0.0, 0.776, 1.0]],
        alpha: [(0.0, 0.0), (0.097, 1.0), (0.835, 1.0), (1.0, 0.0)],
        shrinks: true,
    },
];

#[derive(Clone, Copy, Debug)]
struct Particle {
    pos: Vec2,
    vel: Vec2,
    age: f32,
    life: f32,
    size: f32,
    channel: usize,
}

struct SpotState {
    progress: f32,
    counters: [f32; 3],
}

struct Field {
    particles: Vec<Particle>,
    spots: HashMap<PheromoneId, SpotState>,
    last_time: Option<f64>,
    rng: u64,
}

// One field per thread: the scene is built on the thread that owns the world, whether that is the window's or the simulation worker's.
thread_local! {
    static FIELD: RefCell<Field> = RefCell::new(Field {
        particles: Vec::new(),
        spots: HashMap::new(),
        last_time: None,
        rng: 0x9E37_79B9_7F4A_7C15,
    });
}

impl Field {
    fn random(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        ((self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
    }

    fn between(&mut self, range: (f32, f32)) -> f32 {
        range.0 + (range.1 - range.0) * self.random()
    }

    /// `ParticleSystem.Emit(n)` of a colour at a spot: a cone of 25 degrees over a base of radius 1, aimed along the camera's axis.
    fn emit(&mut self, channel: usize, at: Vec2, count: usize) {
        let system = &SYSTEMS[channel];
        for _ in 0..count {
            if self.particles.len() >= MAX_PARTICLES {
                return;
            }
            // A point of the base disc and the angle off the axis that grows with its distance from the middle.
            let fraction = self.random().sqrt();
            let azimuth = self.random() * std::f32::consts::TAU;
            let theta = 25.0_f32.to_radians() * fraction;
            let speed = self.between(system.speed);
            let sideways = speed * theta.sin();
            let direction = Vec2::new(azimuth.cos(), azimuth.sin());
            let life = self.between(system.life);
            let size = self.between(system.size);
            self.particles.push(Particle {
                pos: at + direction * fraction,
                vel: direction * sideways,
                age: 0.0,
                life,
                size,
                channel,
            });
        }
    }
}

/// Advances the particles to the world's clock, emitting for every spot as `PheromoneSpot.FixedUpdate` does. Called once per drawn frame.
/// `strength` is the player's "Pheromones Strength" (0..1); with `show` off nothing is emitted and what is alive goes (`ParticlesMaster`).
pub fn advance(world: &World, show: bool, strength: f32) {
    FIELD.with(|cell| advance_field(&mut cell.borrow_mut(), world, show, strength));
}

fn advance_field(field: &mut Field, world: &World, show: bool, strength: f32) {
    let now = world.simulated_time();
    let dt = match field.last_time {
        Some(before) if now >= before => (now - before) as f32,
        _ => 0.0,
    };
    field.last_time = Some(now);
    // `ParticlesMaster` only stops emitting when the setting is off; what is in the air finishes its life.
    // A long step (a fast forward, a load) ages what is in the air by the whole of it: a particle lives a few seconds of game time, so a
    // step over a second used to be skipped ("not played back") and at a high speed the particles stayed for ever.
    if dt > 0.0 {
        for particle in &mut field.particles {
            particle.age += dt;
            particle.pos += particle.vel * dt;
        }
        field.particles.retain(|p| p.age < p.life);
    }
    // The spots that are gone take their counters with them.
    if !show {
        field.spots.clear();
        return;
    }
    let ids: Vec<PheromoneId> = world.pheromones.ids().collect();
    let alive: std::collections::HashSet<PheromoneId> = ids.iter().copied().collect();
    field.spots.retain(|id, _| alive.contains(id));
    for id in ids {
        let Some(spot) = world.pheromones.get(id) else {
            continue;
        };
        if !field.spots.contains_key(&id) {
            // `InitPheromones`: `sqrt(strength) * Random.Range(0.5, 1.5)` for each colour (nothing when the strength is tiny).
            let mut counters = [0.0; 3];
            if strength > 0.01 {
                for counter in &mut counters {
                    *counter = strength.sqrt() * (0.5 + field.random());
                }
            }
            field.spots.insert(id, SpotState { progress: 0.0, counters });
        }
        let Some(state) = field.spots.get_mut(&id) else {
            continue;
        };
        state.progress += dt;
        if state.progress < PERIOD {
            continue;
        }
        let quanta = (state.progress / PERIOD).floor() * PERIOD;
        state.progress -= quanta;
        let mut counters = state.counters;
        for (channel, counter) in counters.iter_mut().enumerate() {
            if spot.strength[channel] > 0.0 {
                *counter += COUNT_PER_PERIOD * strength;
            }
        }
        let mut emit = [0usize; 3];
        for (channel, counter) in counters.iter_mut().enumerate() {
            if *counter >= 1.0 {
                emit[channel] = *counter as usize;
            }
            *counter %= 1.0;
        }
        if let Some(state) = field.spots.get_mut(&id) {
            state.counters = counters;
        }
        for (channel, count) in emit.into_iter().enumerate() {
            if count > 0 {
                field.emit(channel, spot.pos, count);
            }
        }
    }
}

/// A particle as it is drawn now: its centre, the half diagonal of its diamond and its colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    /// Centre.
    pub pos: Vec2,
    /// Half of the diagonal of the quad (a square turned 45 degrees: its side over the square root of two).
    pub half_diagonal: f32,
    /// RGBA.
    pub color: [u8; 4],
}

fn alpha_of(keys: &[(f32, f32); 4], t: f32) -> f32 {
    for pair in keys.windows(2) {
        let ((t0, a0), (t1, a1)) = (pair[0], pair[1]);
        if t <= t1 {
            return if t1 > t0 { a0 + (a1 - a0) * ((t - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { a1 };
        }
    }
    keys[3].1
}

/// What a particle looks like at a point of its life (`ColorModule`, `SizeModule`).
fn look(particle: &Particle) -> ([u8; 4], f32) {
    let system = &SYSTEMS[particle.channel];
    let t = (particle.age / particle.life).clamp(0.0, 1.0);
    let mix = |a: f32, b: f32| a + (b - a) * t;
    let rgb = [
        mix(system.colors[0][0], system.colors[1][0]),
        mix(system.colors[0][1], system.colors[1][1]),
        mix(system.colors[0][2], system.colors[1][2]),
    ];
    let alpha = alpha_of(&system.alpha, t);
    let size = if system.shrinks { particle.size * (1.0 - t) } else { particle.size };
    (
        [
            (rgb[0] * 255.0).round() as u8,
            (rgb[1] * 255.0).round() as u8,
            (rgb[2] * 255.0).round() as u8,
            (alpha * 255.0).round() as u8,
        ],
        size,
    )
}

/// The particles inside a world rectangle, oldest first (the game sorts transparent particles by distance; they all fly away from the camera at the same
/// pace, so the oldest are the farthest).
pub fn drawn(view_min: Vec2, view_max: Vec2) -> Vec<Drawn> {
    FIELD.with(|cell| drawn_of(&cell.borrow(), view_min, view_max))
}

fn drawn_of(field: &Field, view_min: Vec2, view_max: Vec2) -> Vec<Drawn> {
    let mut out = Vec::new();
    for particle in &field.particles {
        let (color, size) = look(particle);
        if color[3] == 0 || size <= 0.0 {
            continue;
        }
        let reach = size;
        if particle.pos.x + reach < view_min.x
            || particle.pos.x - reach > view_max.x
            || particle.pos.y + reach < view_min.y
            || particle.pos.y - reach > view_max.y
        {
            continue;
        }
        out.push(Drawn {
            pos: particle.pos,
            half_diagonal: size * std::f32::consts::FRAC_1_SQRT_2,
            color,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_particle_fades_in_holds_and_fades_out() {
        let mut particle = Particle {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            age: 0.0,
            life: 2.0,
            size: 1.0,
            channel: 0,
        };
        assert_eq!(look(&particle).0[3], 0, "invisible at birth");
        particle.age = 1.0;
        assert_eq!(look(&particle).0[3], 255, "fully there in the middle");
        particle.age = 1.99;
        assert!(look(&particle).0[3] < 20, "nearly gone at the end");
        // Red runs from red to orange.
        assert_eq!(look(&particle).0[0], 255);
        particle.age = 1.0;
        let middle = look(&particle).0;
        assert!(middle[1] > 60 && middle[1] < 100, "{middle:?}");
    }

    #[test]
    fn the_blue_particle_shrinks_and_the_red_one_does_not() {
        let mk = |channel| Particle {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            age: 1.0,
            life: 2.0,
            size: 2.0,
            channel,
        };
        assert!((look(&mk(2)).1 - 1.0).abs() < 1e-5);
        assert!((look(&mk(0)).1 - 2.0).abs() < 1e-5);
    }

    #[test]
    fn an_emission_is_a_cone_that_leaves_sideways_at_most_sin_25_of_the_speed() {
        let mut field = Field {
            particles: Vec::new(),
            spots: HashMap::new(),
            last_time: None,
            rng: 12345,
        };
        field.emit(0, Vec2::new(10.0, 10.0), 500);
        assert_eq!(field.particles.len(), 500);
        for p in &field.particles {
            assert!(p.vel.length() <= 12.0 * 25.0_f32.to_radians().sin() + 1e-4);
            assert!((p.pos - Vec2::new(10.0, 10.0)).length() <= 1.0 + 1e-4);
            assert!((1.0..=2.0).contains(&p.size) && (2.0..=3.0).contains(&p.life));
        }
    }
}
