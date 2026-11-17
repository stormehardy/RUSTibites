//! The blood of a body that bursts into meat (`BibiteBody.ExplodeToMeat` -> `ParticlesMaster.BloodSplatterAtPosition` -> `BloodBurst`, the `blood`
//! `ParticleSystem` of the prefab).
//!
//! The coroutine emits `(int)baseBodyArea / 20` particles five times, 0.05 s apart, over a disc of radius `bodyLength / 2` (a circle shape with its whole
//! area filled, `radiusThickness` 1). A particle has no start speed; the velocity-over-lifetime module only moves it away from the middle (radial speed
//! `4.6157 * curve(t)` times the speed modifier 2, the curve easing from 1 to 0). It lives 0.5 s and is a 7 unit square of the `blood particles` sheet (10 x 4
//! tiles of 7 pixels: a splat that grows over ten frames, one row of four picked at random per particle) whose frame follows a curve over its life (it
//! grows to the biggest splat in the first seventh, stays near it and shrinks to nothing). The material's colour mode is `Color`: the pixel keeps the
//! particle colour's hue and saturation and the texture's brightness. The particle colour is the colour-over-lifetime gradient (dark red until 0.79 of the
//! life, then towards pink-red) and its alpha is 0.81 until 0.78 of the life and then fades to 0.
//!
//! The state is only for the eye: it lives here, advances with the world's own clock (a pause stops it) and is not saved.

use std::cell::RefCell;

use bibites_core::math::Vec2;
use bibites_core::world::World;

use crate::blood_sheet as sheet;

/// `startLifetime`.
const LIFE: f32 = 0.5;
/// `startSize`: the side of the square, in world units (a tile has 7 pixels).
const SIZE: f32 = 7.0;
/// The coroutine's emissions and the time between them.
const EMISSIONS: usize = 5;
const DELAY: f64 = 0.05;
/// The radial speed of `VelocityModule` (its curve's scale) and `speedModifier`.
const RADIAL: f32 = 4.615_722_7 * 2.0;
/// A cap on what is alive at once.
const MAX_PARTICLES: usize = 20_000;

#[derive(Clone, Copy, Debug)]
struct Particle {
    pos: Vec2,
    dir: Vec2,
    age: f32,
    row: usize,
}

#[derive(Clone, Copy, Debug)]
struct Emission {
    due: f64,
    pos: Vec2,
    radius: f32,
    count: usize,
}

struct Field {
    particles: Vec<Particle>,
    pending: Vec<Emission>,
    seen: u64,
    last_time: Option<f64>,
    rng: u64,
}

thread_local! {
    static FIELD: RefCell<Field> = RefCell::new(Field {
        particles: Vec::new(),
        pending: Vec::new(),
        seen: 0,
        last_time: None,
        rng: 0xD1B5_4A32_D192_ED03,
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

    /// `ParticleSystem.Emit(count)` over the disc: a point of its area, moving outwards, one of the four rows of the sheet.
    fn emit(&mut self, at: Vec2, radius: f32, count: usize) {
        for _ in 0..count {
            if self.particles.len() >= MAX_PARTICLES {
                return;
            }
            let distance = radius * self.random().sqrt();
            let azimuth = self.random() * std::f32::consts::TAU;
            let dir = Vec2::new(azimuth.cos(), azimuth.sin());
            let row = ((self.random() * 4.0) as usize).min(3);
            self.particles.push(Particle { pos: at + dir * distance, dir, age: 0.0, row });
        }
    }
}

/// A cubic Hermite curve through keys `(time, value, in slope, out slope)` (Unity's `AnimationCurve`).
fn curve(keys: &[(f32, f32, f32, f32)], t: f32) -> f32 {
    let first = keys[0];
    if t <= first.0 {
        return first.1;
    }
    for pair in keys.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t <= b.0 {
            let span = b.0 - a.0;
            let u = (t - a.0) / span;
            let (u2, u3) = (u * u, u * u * u);
            return (2.0 * u3 - 3.0 * u2 + 1.0) * a.1
                + (u3 - 2.0 * u2 + u) * span * a.3
                + (-2.0 * u3 + 3.0 * u2) * b.1
                + (u3 - u2) * span * b.2;
        }
    }
    keys[keys.len() - 1].1
}

/// `VelocityModule.radial`: 1 at birth, 0 at the end of the life.
const RADIAL_CURVE: [(f32, f32, f32, f32); 2] = [(0.0, 1.0, 0.0, 0.0), (1.0, 0.0, -1.715_328, -1.715_328)];
/// `TextureSheetAnimation.frameOverTime`: the share of the ten frames.
const FRAME_CURVE: [(f32, f32, f32, f32); 4] = [
    (0.0, 0.507_293_7, 1.765_511, 3.453_98),
    (0.142_648_85, 1.0, 3.453_98, -0.562_709_8),
    (0.661_487_9, 0.708_044_2, -0.562_709_8, -2.091_635_7),
    (1.0, 0.0, -2.091_635_7, 3.161_496),
];

/// Advances the blood to the world's clock and takes in the bursts of the bodies that exploded since the last call.
pub fn advance(world: &World) {
    FIELD.with(|cell| advance_field(&mut cell.borrow_mut(), world));
}

fn advance_field(field: &mut Field, world: &World) {
    let now = world.simulated_time();
    let step = match field.last_time {
        Some(before) if now >= before => now - before,
        _ => 0.0,
    };
    field.last_time = Some(now);
    if step >= f64::from(LIFE) {
        // A step as long as a particle's life or longer (a fast forward: at a high time scale a frame steps more than a second of
        // game time, and a load jumps further): everything that was in the air is past its life by the end of it, as it is for
        // Unity's particle system, which ages by the frame's scaled time. (This used to skip the ageing for a step over a second, "not
        // played back", so at a high speed the blood stayed on the screen for ever.)
        field.particles.clear();
    } else if step > 0.0 {
        let dt = step as f32;
        for particle in &mut field.particles {
            let t = (particle.age / LIFE).clamp(0.0, 1.0);
            particle.pos += particle.dir * (RADIAL * curve(&RADIAL_CURVE, t) * dt);
            particle.age += dt;
        }
        field.particles.retain(|p| p.age < LIFE);
    }
    let mut newest = field.seen;
    for (number, burst) in world.bibites.blood_log() {
        if *number <= field.seen {
            continue;
        }
        newest = newest.max(*number);
        if burst.count > 0 {
            for k in 0..EMISSIONS {
                field.pending.push(Emission {
                    due: now + DELAY * k as f64,
                    pos: burst.pos,
                    radius: burst.radius,
                    count: burst.count as usize,
                });
            }
        }
    }
    field.seen = newest;
    let mut index = 0;
    while index < field.pending.len() {
        if field.pending[index].due <= now {
            let emission = field.pending.swap_remove(index);
            field.emit(emission.pos, emission.radius, emission.count);
        } else {
            index += 1;
        }
    }
}

/// One lit pixel of a splat: its rectangle in world units and its colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pixel {
    /// Lower-left corner.
    pub min: Vec2,
    /// Upper-right corner.
    pub max: Vec2,
    /// RGBA.
    pub color: [u8; 4],
}

/// `ColorModule`: the colour (and alpha) over the life, as it is before the material's colour mode.
fn gradient(t: f32) -> ([f32; 3], f32) {
    const A: [f32; 3] = [0.622_641_5, 0.049_928_8, 0.067_080_5];
    const B: [f32; 3] = [1.0, 0.0, 0.126_577_85];
    const COLOR_KEY: f32 = 51_657.0 / 65_535.0;
    const ALPHA_KEY: f32 = 51_079.0 / 65_535.0;
    let c = if t <= COLOR_KEY {
        A
    } else {
        let u = (t - COLOR_KEY) / (1.0 - COLOR_KEY);
        [A[0] + (B[0] - A[0]) * u, A[1] + (B[1] - A[1]) * u, A[2] + (B[2] - A[2]) * u]
    };
    let alpha = if t <= ALPHA_KEY { 0.807_843_1 } else { 0.807_843_1 * (1.0 - (t - ALPHA_KEY) / (1.0 - ALPHA_KEY)) };
    (c, alpha.max(0.0))
}

/// The material's `_ColorMode` `Color`: the hue and saturation of the particle colour with the brightness of the texture.
fn colored(c: [f32; 3], value: f32) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let delta = max - min;
    let saturation = if max > 0.0 { delta / max } else { 0.0 };
    let hue = if delta <= 0.0 {
        0.0
    } else if max == c[0] {
        ((c[1] - c[2]) / delta).rem_euclid(6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / delta + 2.0
    } else {
        (c[0] - c[1]) / delta + 4.0
    };
    let chroma = value * saturation;
    let x = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = value - chroma;
    [r + m, g + m, b + m]
}

/// The lit pixels of every splat inside a world rectangle (the game sorts them by distance; they do not overlap in a way that shows).
pub fn drawn(view_min: Vec2, view_max: Vec2) -> Vec<Pixel> {
    FIELD.with(|cell| drawn_of(&cell.borrow(), view_min, view_max))
}

fn drawn_of(field: &Field, view_min: Vec2, view_max: Vec2) -> Vec<Pixel> {
    let mut out = Vec::new();
    let half = SIZE / 2.0;
    for particle in &field.particles {
        if particle.pos.x + half < view_min.x
            || particle.pos.x - half > view_max.x
            || particle.pos.y + half < view_min.y
            || particle.pos.y - half > view_max.y
        {
            continue;
        }
        let t = (particle.age / LIFE).clamp(0.0, 1.0);
        let frame = ((curve(&FRAME_CURVE, t).clamp(0.0, 1.0) * 10.0) as usize).min(9);
        let (tint, alpha) = gradient(t);
        let alpha = (alpha * 255.0).round() as u8;
        if alpha == 0 {
            continue;
        }
        for py in 0..7 {
            for px in 0..7 {
                let at = (particle.row * 7 + py) * sheet::WIDTH + frame * 7 + px;
                if sheet::ALPHA[at] < 128 {
                    continue;
                }
                let rgb = colored(tint, f32::from(sheet::VALUE[at]) / 255.0);
                let x = particle.pos.x - half + px as f32;
                let y = particle.pos.y + half - (py + 1) as f32;
                out.push(Pixel {
                    min: Vec2::new(x, y),
                    max: Vec2::new(x + 1.0, y + 1.0),
                    color: [
                        (rgb[0] * 255.0).round() as u8,
                        (rgb[1] * 255.0).round() as u8,
                        (rgb[2] * 255.0).round() as u8,
                        alpha,
                    ],
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field_with(particle: Particle) -> Field {
        Field { particles: vec![particle], pending: Vec::new(), seen: 0, last_time: None, rng: 1 }
    }

    #[test]
    fn the_splat_grows_to_the_biggest_frame_and_shrinks_away() {
        let frame = |t: f32| ((curve(&FRAME_CURVE, t).clamp(0.0, 1.0) * 10.0) as usize).min(9);
        assert_eq!(frame(0.0), 5, "it starts half grown");
        assert_eq!(frame(0.1426), 9, "the biggest splat early on");
        assert!(frame(0.66) >= 7);
        assert_eq!(frame(1.0), 0, "nothing at the end");
    }

    #[test]
    fn the_colour_keeps_the_particles_hue_and_the_textures_brightness() {
        // Dark red (0.62, 0.05, 0.07) over a full-brightness texel is a bright red with a touch of blue.
        let bright = colored([0.622_641_5, 0.049_928_8, 0.067_080_5], 1.0);
        assert!((bright[0] - 1.0).abs() < 1e-5 && bright[1] < 0.15 && bright[2] < 0.2, "{bright:?}");
        let dim = colored([0.622_641_5, 0.049_928_8, 0.067_080_5], 0.5);
        assert!((dim[0] - 0.5).abs() < 1e-5);
        // A grey stays grey.
        assert_eq!(colored([0.4, 0.4, 0.4], 0.8), [0.8, 0.8, 0.8]);
    }

    #[test]
    fn the_alpha_holds_then_fades_and_the_particle_moves_outwards_and_stops() {
        assert!((gradient(0.5).1 - 0.807_843_1).abs() < 1e-5);
        assert!(gradient(0.9).1 < 0.5 && gradient(1.0).1 == 0.0);
        // The radial speed goes from `9.23` to nothing.
        assert!((RADIAL * curve(&RADIAL_CURVE, 0.0) - 9.231_445).abs() < 1e-3);
        assert!(curve(&RADIAL_CURVE, 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_particle_is_drawn_inside_its_seven_unit_square_and_only_while_it_lives() {
        let particle = Particle { pos: Vec2::new(10.0, 20.0), dir: Vec2::new(1.0, 0.0), age: 0.1, row: 2 };
        let field = field_with(particle);
        let pixels = drawn_of(&field, Vec2::new(-100.0, -100.0), Vec2::new(100.0, 100.0));
        assert!(!pixels.is_empty());
        for pixel in &pixels {
            assert!(pixel.min.x >= 6.4 && pixel.max.x <= 13.6 && pixel.min.y >= 16.4 && pixel.max.y <= 23.6, "{pixel:?}");
            assert_eq!(pixel.color[3], 206);
        }
        // Out of view: nothing.
        assert!(drawn_of(&field, Vec2::new(100.0, 100.0), Vec2::new(200.0, 200.0)).is_empty());
    }
}
