//! RUSTibites Improved: streams.
//!
//! A stream is a band of the world the player defines in the Zones tab of the world's parameters panel (its "Streams" section): two points, a **start** and a **finish**
//! (in the world's x and y), a **width** and a **strength**. Everything inside the band, bibites (the dead ones too), pellets that no mouth holds and eggs, is pushed in the
//! direction from the start to the finish: every tick the stream adds `strength * dt` to the velocity of each body in it, an acceleration like the force tower's (a heavy body
//! and a light one gain the same speed), and the world's own drag and collisions do the rest, so a body in a stream settles at a speed of about `strength / drag`.
//!
//! The band is the rectangle that has the segment from the start to the finish as its middle line and the width across it: it begins at the start and ends at the finish
//! (a body past either end is out of it), and the push is the same all the way across and along. Two streams that overlap push by the sum of their pushes.
//!
//! The game has no such thing, so a stream is never written into a save (the viewer keeps them in a file beside the save) and it acts only while
//! [`crate::improved::Feature::Streams`] is on. It draws nothing from the world's random numbers; with no stream in a world nothing of this runs.

use crate::math::Vec2;

/// The narrowest stream (u).
pub const STREAM_WIDTH_MIN: f32 = 10.0;
/// The widest stream (u).
pub const STREAM_WIDTH_MAX: f32 = 3000.0;
/// The width of a new stream (u).
pub const STREAM_WIDTH_DEFAULT: f32 = 100.0;
/// The weakest stream (u/s per second): none at all.
pub const STREAM_STRENGTH_MIN: f32 = 0.0;
/// The strongest stream (u/s per second).
pub const STREAM_STRENGTH_MAX: f32 = 500.0;
/// The strength of a new stream (u/s per second).
pub const STREAM_STRENGTH_DEFAULT: f32 = 100.0;
/// A stream whose start and finish are closer than this (u) has no direction: it pushes nothing.
pub const STREAM_MIN_LENGTH: f32 = 1.0;

/// A stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stream {
    /// Where it begins (world x, y).
    pub start: Vec2,
    /// Where it ends (world x, y); the push is towards it.
    pub end: Vec2,
    /// How wide the band is, in u ([`STREAM_WIDTH_MIN`] to [`STREAM_WIDTH_MAX`]).
    pub width: f32,
    /// The acceleration it gives, in u/s per second ([`STREAM_STRENGTH_MIN`] to [`STREAM_STRENGTH_MAX`]).
    pub strength: f32,
}

impl Stream {
    /// A stream from `start` to `end` with the default width and strength.
    #[must_use]
    pub fn new(start: Vec2, end: Vec2) -> Self {
        let mut stream = Self { start, end, width: STREAM_WIDTH_DEFAULT, strength: STREAM_STRENGTH_DEFAULT };
        stream.set_start(start);
        stream.set_end(end);
        stream
    }

    /// Moves the start (a value that is not a number is ignored).
    pub fn set_start(&mut self, start: Vec2) {
        if start.is_finite() {
            self.start = start;
        }
    }

    /// Moves the finish (a value that is not a number is ignored).
    pub fn set_end(&mut self, end: Vec2) {
        if end.is_finite() {
            self.end = end;
        }
    }

    /// Sets the width (kept inside its range; a value that is not a number is ignored).
    pub fn set_width(&mut self, width: f32) {
        if width.is_finite() {
            self.width = width.clamp(STREAM_WIDTH_MIN, STREAM_WIDTH_MAX);
        }
    }

    /// Sets the strength (kept inside its range; a value that is not a number is ignored).
    pub fn set_strength(&mut self, strength: f32) {
        if strength.is_finite() {
            self.strength = strength.clamp(STREAM_STRENGTH_MIN, STREAM_STRENGTH_MAX);
        }
    }

    /// How far it is from the start to the finish (u).
    #[must_use]
    pub fn length(&self) -> f32 {
        (self.end - self.start).length()
    }

    /// The unit vector from the start to the finish; `None` when they are closer than [`STREAM_MIN_LENGTH`].
    #[must_use]
    pub fn direction(&self) -> Option<Vec2> {
        let along = self.end - self.start;
        let length = along.length();
        (length.is_finite() && length >= STREAM_MIN_LENGTH).then(|| along * (1.0 / length))
    }

    /// Whether `point` is in the band: between the start and the finish along it, and no farther than half the width from its middle line.
    #[must_use]
    pub fn contains(&self, point: Vec2) -> bool {
        let Some(direction) = self.direction() else {
            return false;
        };
        let from_start = point - self.start;
        let along = from_start.dot(direction);
        let across = direction.cross(from_start);
        along.is_finite() && across.is_finite() && along >= 0.0 && along <= self.length() && across.abs() <= self.width * 0.5
    }

    /// The change of velocity a body at `at` gets from `dt` seconds of the stream: `strength * dt` towards the finish when it is in the band, nothing otherwise.
    #[must_use]
    pub fn kick(&self, at: Vec2, dt: f32) -> Vec2 {
        if !(dt > 0.0 && dt.is_finite() && self.strength > 0.0) || !self.contains(at) {
            return Vec2::ZERO;
        }
        self.direction().map_or(Vec2::ZERO, |direction| direction * (self.strength * dt))
    }

    /// The four corners of the band, going round it: the start's two, then the finish's two (`None` without a direction). What the viewer draws.
    #[must_use]
    pub fn corners(&self) -> Option<[Vec2; 4]> {
        let direction = self.direction()?;
        let side = direction.perp() * (self.width * 0.5);
        Some([self.start + side, self.end + side, self.end - side, self.start - side])
    }

    /// Whether the stream pushes anything at all (a strength of 0, or a start on the finish, leaves the world as it is).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.strength > 0.0 && self.direction().is_some()
    }

    /// The stream with what the test of a body needs worked out once for the tick of `dt` seconds (the direction, the length, the push), for a pass that asks it of
    /// many bodies; `None` when the stream pushes nothing in that tick ([`Stream::kick`] is zero everywhere then).
    #[must_use]
    pub fn prepare(&self, dt: f32) -> Option<StreamKick> {
        if !(dt > 0.0 && dt.is_finite() && self.strength > 0.0) {
            return None;
        }
        let direction = self.direction()?;
        Some(StreamKick { start: self.start, direction, length: self.length(), half_width: self.width * 0.5, push: direction * (self.strength * dt) })
    }
}

/// A stream prepared for one tick (see [`Stream::prepare`]): [`StreamKick::kick`] gives exactly, bit for bit, what [`Stream::kick`] gives for that tick, without working out
/// the direction and the length again for every body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamKick {
    start: Vec2,
    direction: Vec2,
    length: f32,
    half_width: f32,
    push: Vec2,
}

impl StreamKick {
    /// The change of velocity a body at `at` gets: the push when it is in the band, nothing otherwise.
    #[inline]
    #[must_use]
    pub fn kick(&self, at: Vec2) -> Vec2 {
        let from_start = at - self.start;
        let along = from_start.dot(self.direction);
        let across = self.direction.cross(from_start);
        if along.is_finite() && across.is_finite() && along >= 0.0 && along <= self.length && across.abs() <= self.half_width {
            self.push
        } else {
            Vec2::ZERO
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream() -> Stream {
        // From (-100, 50) to (300, 50): 400 long, 100 wide, strength 100.
        Stream::new(Vec2::new(-100.0, 50.0), Vec2::new(300.0, 50.0))
    }

    #[test]
    fn a_prepared_stream_kicks_exactly_as_the_stream_does() {
        // Slanted, long, short, wide, narrow, weak, strong, one without a direction: over a grid of points that crosses every edge of every band, and for the steps that count.
        let streams = [
            stream(),
            Stream::new(Vec2::new(-300.0, -120.0), Vec2::new(410.0, 333.0)),
            Stream::new(Vec2::new(5.0, 5.0), Vec2::new(6.0, 5.5)),
            Stream::new(Vec2::new(1000.0, -2000.0), Vec2::new(-999.0, 2001.0)),
            Stream::new(Vec2::new(0.0, 0.0), Vec2::new(0.0, 0.0)),
        ];
        for (index, mut stream) in streams.into_iter().enumerate() {
            stream.set_width([100.0, 10.0, 3000.0, 777.7, 50.0][index]);
            stream.set_strength([100.0, 3.3, 500.0, 0.0, 250.0][index]);
            // The edges of the band: its start and finish, its corners and the middle of its sides, where a `<=` against a `<` would show.
            let mut edge_points = vec![stream.start, stream.end];
            if let Some(corners) = stream.corners() {
                edge_points.extend(corners);
                edge_points.extend((0..4).map(|i| (corners[i] + corners[(i + 1) % 4]) * 0.5));
            }
            for dt in [0.025_f32, 1.0 / 30.0, 0.0, -1.0, f32::NAN, f32::INFINITY] {
                let prepared = stream.prepare(dt);
                for at in &edge_points {
                    let (expected, got) = (stream.kick(*at, dt), prepared.map_or(Vec2::ZERO, |kick| kick.kick(*at)));
                    assert_eq!((expected.x.to_bits(), expected.y.to_bits()), (got.x.to_bits(), got.y.to_bits()), "stream {index}, dt {dt}, at the edge point {at:?}");
                }
                for ix in -60..=60 {
                    for iy in -60..=60 {
                        let at = Vec2::new(ix as f32 * 41.3 + 0.17, iy as f32 * 39.9 - 0.31);
                        let expected = stream.kick(at, dt);
                        let got = prepared.map_or(Vec2::ZERO, |kick| kick.kick(at));
                        assert_eq!((expected.x.to_bits(), expected.y.to_bits()), (got.x.to_bits(), got.y.to_bits()), "stream {index}, dt {dt}, at {at:?}");
                    }
                }
            }
        }
        // A point that is not a number is in no band, prepared or not.
        let prepared = stream().prepare(0.025).expect("it pushes");
        assert_eq!(prepared.kick(Vec2::new(f32::NAN, 0.0)), Vec2::ZERO);
        assert_eq!(stream().kick(Vec2::new(f32::NAN, 0.0), 0.025), Vec2::ZERO);
    }

    #[test]
    fn a_new_stream_has_the_default_width_and_strength_and_a_direction_from_start_to_finish() {
        let stream = stream();
        assert_eq!((stream.width, stream.strength), (STREAM_WIDTH_DEFAULT, STREAM_STRENGTH_DEFAULT));
        assert_eq!(stream.length(), 400.0);
        assert_eq!(stream.direction(), Some(Vec2::new(1.0, 0.0)));
        assert!(stream.is_active());
        // From the finish to the start it runs the other way.
        let back = Stream::new(stream.end, stream.start);
        assert_eq!(back.direction(), Some(Vec2::new(-1.0, 0.0)));
    }

    #[test]
    fn the_setters_keep_every_value_inside_its_range_and_ignore_what_is_not_a_number() {
        let mut stream = stream();
        stream.set_width(1.0e9);
        assert_eq!(stream.width, STREAM_WIDTH_MAX);
        stream.set_width(0.0);
        assert_eq!(stream.width, STREAM_WIDTH_MIN);
        stream.set_width(f32::NAN);
        assert_eq!(stream.width, STREAM_WIDTH_MIN, "not a number: unchanged");
        stream.set_strength(-5.0);
        assert_eq!(stream.strength, STREAM_STRENGTH_MIN);
        assert!(!stream.is_active(), "a strength of 0 does nothing");
        stream.set_strength(1.0e9);
        assert_eq!(stream.strength, STREAM_STRENGTH_MAX);
        stream.set_strength(f32::INFINITY);
        assert_eq!(stream.strength, STREAM_STRENGTH_MAX);
        let before = (stream.start, stream.end);
        stream.set_start(Vec2::new(f32::NAN, 0.0));
        stream.set_end(Vec2::new(0.0, f32::INFINITY));
        assert_eq!((stream.start, stream.end), before);
    }

    #[test]
    fn the_band_is_the_rectangle_round_the_middle_line_from_the_start_to_the_finish() {
        let stream = stream();
        // Inside: the middle, and just inside each edge and end.
        for point in [Vec2::new(100.0, 50.0), Vec2::new(-99.0, 50.0), Vec2::new(299.0, 50.0), Vec2::new(100.0, 99.0), Vec2::new(100.0, 1.0)] {
            assert!(stream.contains(point), "{point:?}");
        }
        // Outside: past either end, and past either side (half the width is 50).
        for point in [Vec2::new(-101.0, 50.0), Vec2::new(301.0, 50.0), Vec2::new(100.0, 101.0), Vec2::new(100.0, -1.0), Vec2::new(1000.0, 1000.0)] {
            assert!(!stream.contains(point), "{point:?}");
        }
        // A band at an angle: from the origin to (100, 100), 20 wide.
        let mut slanted = Stream::new(Vec2::ZERO, Vec2::new(100.0, 100.0));
        slanted.set_width(20.0);
        assert!(slanted.contains(Vec2::new(50.0, 50.0)));
        assert!(slanted.contains(Vec2::new(55.0, 45.0)), "5 u off the line is inside a width of 20");
        assert!(!slanted.contains(Vec2::new(70.0, 40.0)), "21 u off the line is outside it");
        assert!(!slanted.contains(Vec2::new(-5.0, -5.0)), "behind the start");
        assert!(!slanted.contains(Vec2::new(110.0, 110.0)), "past the finish");
    }

    #[test]
    fn a_body_in_the_band_is_pushed_towards_the_finish_by_the_strength_times_the_tick_and_one_outside_is_not() {
        let stream = stream();
        let dt = 1.0 / 30.0;
        let kick = stream.kick(Vec2::new(100.0, 60.0), dt);
        assert!((kick.x - 100.0 * dt).abs() < 1e-5 && kick.y.abs() < 1e-6, "{kick:?}");
        assert_eq!(stream.kick(Vec2::new(100.0, 500.0), dt), Vec2::ZERO);
        assert_eq!(stream.kick(Vec2::new(100.0, 60.0), 0.0), Vec2::ZERO);
        assert_eq!(stream.kick(Vec2::new(100.0, 60.0), f32::NAN), Vec2::ZERO);
        // The push does not depend on where in the band the body is.
        assert_eq!(stream.kick(Vec2::new(-90.0, 50.0), dt), stream.kick(Vec2::new(290.0, 80.0), dt));
        // Towards the finish, whichever way that is: a stream running down.
        let down = Stream::new(Vec2::new(0.0, 200.0), Vec2::new(0.0, -200.0));
        let kick = down.kick(Vec2::new(10.0, 0.0), dt);
        assert!(kick.y < 0.0 && kick.x.abs() < 1e-5, "{kick:?}");
    }

    #[test]
    fn a_stream_whose_start_is_on_its_finish_has_no_direction_and_pushes_nothing() {
        let point = Vec2::new(5.0, 5.0);
        let stream = Stream::new(point, point + Vec2::new(0.5, 0.0));
        assert_eq!(stream.direction(), None);
        assert!(!stream.is_active() && !stream.contains(point) && stream.corners().is_none());
        assert_eq!(stream.kick(point, 0.1), Vec2::ZERO);
    }

    #[test]
    fn the_corners_go_round_the_band() {
        let corners = stream().corners().expect("a direction");
        // The start's two corners are at x = -100, the finish's at x = 300, 50 either side of y = 50.
        assert_eq!(corners[0].x, -100.0);
        assert_eq!(corners[3].x, -100.0);
        assert_eq!(corners[1].x, 300.0);
        assert_eq!(corners[2].x, 300.0);
        let mut ys = [corners[0].y, corners[3].y];
        ys.sort_by(f32::total_cmp);
        assert_eq!(ys, [0.0, 100.0]);
    }
}
