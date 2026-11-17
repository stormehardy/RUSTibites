//! The history of one zone (`Here/Utility.DataLogging/LogLikeZoneDataArray.cs`, `ZoneDataPoint.cs`,
//! `Utility/LogLikePresenceArray.cs`).
//!
//! Every zone logs a [`ZoneDataPoint`] each time the species log runs (`DataLogger.FixedUpdate` calls
//! `GlobalLineageManager.LogSpeciesInfo` and `ZoneManager.LogAllZonesData` together: every 5 simulated
//! minutes), into a log-like array of the same shape as the species' ([`crate::species_log`]): the newest
//! points at full resolution and older ones compressed in blocks into coarser scales. A zone is present in a
//! point while it has a radius and a fertility. The species Position tab reads it to draw where the zones
//! were when the species was around.
//!
//! Saved in `data.bin` (the fourth block, `ZoneManager.SaveStateBin`): for each zone its id, the length of its
//! log and the log. The points are stored as they are (positions relative to the simulation size, quantised
//! to a short), so a log that is loaded and saved again does not change.

use crate::error::{Error, Result};
use crate::species_log::{LAST, LAYOUT, N_SCALES};

/// One logged sample of a zone (`ZoneDataPoint`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ZoneDataPoint {
    /// `posX`: the relative position (`ZoneSettings.posX`, kept within +-0.9999) times 32767.
    pub pos_x: i16,
    /// `posY`.
    pub pos_y: i16,
    /// `radius`: the relative radius (half the larger side of a rectangle) times 32767.
    pub radius: u16,
    /// `fertility`: the zone's `totalGrowth` (energy per second).
    pub fertility: f32,
    /// `biomass`: the energy of the pellets the zone owns (`pelletBiomass`).
    pub biomass: f32,
}

impl ZoneDataPoint {
    /// Bytes of a point in `data.bin`: two shorts, a ushort and two floats.
    pub const BYTE_SIZE: usize = 14;

    /// `ZoneDataPoint.present`: the zone had a size and grew something.
    #[must_use]
    pub fn present(&self) -> bool {
        self.radius > 0 && self.fertility > 0.0
    }

    /// `new ZoneDataPoint(Zone)`: `pos` is the relative position setting, `radius` the relative radius (of a
    /// rectangle: half its larger relative side), `total_growth` and `pellet_biomass` the zone's.
    #[must_use]
    pub fn new(pos: [f32; 2], radius: f32, total_growth: f32, pellet_biomass: f32) -> Self {
        let quantise = |value: f32| (value.clamp(-0.9999, 0.9999) * 32767.0) as i16;
        Self {
            pos_x: quantise(pos[0]),
            pos_y: quantise(pos[1]),
            radius: (radius * 32767.0) as u16,
            fertility: total_growth,
            biomass: pellet_biomass,
        }
    }

    /// The position as a fraction of the simulation size (what the game divides by 32767).
    #[must_use]
    pub fn relative_pos(&self) -> [f32; 2] {
        [
            f32::from(self.pos_x) / 32767.0,
            f32::from(self.pos_y) / 32767.0,
        ]
    }

    /// The radius as a fraction of the simulation size.
    #[must_use]
    pub fn relative_radius(&self) -> f32 {
        f32::from(self.radius) / 32767.0
    }
}

/// The history of one zone: a [`ZoneDataPoint`] log-like array that knows where the zone is present.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneLog {
    /// The ring buffers of the finite scales, one after the other (`data`).
    data: Vec<ZoneDataPoint>,
    /// The unbounded last scale, newest first (`infiniteData`).
    infinite: Vec<ZoneDataPoint>,
    /// Where each scale's ring buffer starts (`indexOffsets`).
    index_offsets: [usize; N_SCALES],
    /// How many points each scale handed over since the last compression (`pushCounters`).
    push_counters: [u8; N_SCALES],
    /// How many points of each scale are filled (`nDataAtScale`).
    n_data_at_scale: [u8; N_SCALES],
    /// Present points per scale (`nPresentAtScale`).
    n_present_at_scale: [u16; N_SCALES],
    /// The scale of the oldest present point (`scaleOfApparition`).
    scale_of_apparition: u8,
    /// Its index in that scale (`indexOfApparition`, `-1` before the first point).
    index_of_apparition: i16,
    /// The scale of the newest point that is gone (`scaleOfDisappearance`).
    scale_of_disappearance: u8,
    /// Its index in that scale (`indexOfDisappearance`, `-1` while the newest point is present).
    index_of_disappearance: i16,
}

impl Default for ZoneLog {
    fn default() -> Self {
        Self::new()
    }
}

impl ZoneLog {
    /// `new LogLikeZoneDataArray()`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: vec![ZoneDataPoint::default(); LAYOUT.total],
            infinite: Vec::new(),
            index_offsets: [0; N_SCALES],
            push_counters: [0; N_SCALES],
            n_data_at_scale: [0; N_SCALES],
            n_present_at_scale: [0; N_SCALES],
            scale_of_apparition: 0,
            index_of_apparition: -1,
            scale_of_disappearance: 0,
            index_of_disappearance: -1,
        }
    }

    /// `totalIndexOfApparition`: the global index of the oldest present point (`-1` before the first).
    #[must_use]
    pub fn total_index_of_apparition(&self) -> i32 {
        let before = match self.scale_of_apparition {
            0 => 0,
            scale => LAYOUT.data_sum[usize::from(scale) - 1] as i32,
        };
        i32::from(self.index_of_apparition) + before
    }

    /// `totalIndexOfDisappearance`: the global index of the newest point that is gone, `-1` for a zone that is
    /// present in the newest point.
    #[must_use]
    pub fn total_index_of_disappearance(&self) -> i32 {
        let before = match self.scale_of_disappearance {
            0 => 0,
            scale => LAYOUT.data_sum[usize::from(scale) - 1] as i32,
        };
        i32::from(self.index_of_disappearance) + before
    }

    /// `nPointsPresent`.
    #[must_use]
    pub fn n_points_present(&self) -> u32 {
        self.n_present_at_scale
            .iter()
            .map(|&present| u32::from(present))
            .sum()
    }

    /// `nPointsToSave`: the present points and the ones still waiting to be compressed.
    fn n_points_to_save(&self) -> usize {
        self.n_points_present() as usize
            + self
                .push_counters
                .iter()
                .map(|&count| usize::from(count))
                .sum::<usize>()
    }

    /// `DataSizeAtScale`.
    fn data_size_at_scale(&self, scale: usize) -> usize {
        if scale == LAST {
            self.infinite.len().max(LAYOUT.data_size[LAST - 1])
        } else {
            LAYOUT.data_size[scale]
        }
    }

    /// The point at `index` of `scale` (`this[scale, i]`); a default (absent) point out of range.
    #[must_use]
    pub fn point(&self, scale: usize, index: usize) -> ZoneDataPoint {
        if scale >= N_SCALES {
            return ZoneDataPoint::default();
        }
        if scale == LAST {
            return self.infinite.get(index).copied().unwrap_or_default();
        }
        self.data[self.ring_index(scale, index)]
    }

    /// The point at the global index `index` (`this[i]`: 0 is the newest point), as the species Position tab's
    /// time slider reads it; a default (absent) point past the end.
    #[must_use]
    pub fn at(&self, index: usize) -> ZoneDataPoint {
        let mut index = index;
        for scale in 0..LAST {
            if index < LAYOUT.data_size[scale] {
                return self.point(scale, index);
            }
            index -= LAYOUT.data_size[scale];
        }
        self.infinite.get(index).copied().unwrap_or_default()
    }

    fn ring_index(&self, scale: usize, index: usize) -> usize {
        LAYOUT.scale_offset[scale] + (self.index_offsets[scale] + index) % LAYOUT.scale_size[scale]
    }

    /// Sets a point; false when the index is past the end of the unbounded scale.
    fn set_point(&mut self, scale: usize, index: usize, point: ZoneDataPoint) -> bool {
        if scale == LAST {
            return match index.cmp(&self.infinite.len()) {
                std::cmp::Ordering::Less => {
                    self.infinite[index] = point;
                    true
                }
                std::cmp::Ordering::Equal => {
                    self.infinite.push(point);
                    true
                }
                std::cmp::Ordering::Greater => false,
            };
        }
        let slot = self.ring_index(scale, index);
        self.data[slot] = point;
        true
    }

    /// `Push`: logs a point. The newest point goes to scale 0; the points that age out of a scale are
    /// compressed into the next one.
    pub fn push(&mut self, point: ZoneDataPoint) {
        let mut value = point;
        for scale in 0..LAST {
            let scale_size = LAYOUT.scale_size[scale];
            let data_size = LAYOUT.data_size[scale];
            let offset = match self.index_offsets[scale] {
                0 => scale_size - 1,
                offset => offset - 1,
            };
            self.index_offsets[scale] = offset;
            self.data[LAYOUT.scale_offset[scale] + offset] = value;
            self.note_pushed(&value, scale);
            if usize::from(self.n_data_at_scale[scale]) < data_size {
                self.n_data_at_scale[scale] += 1;
                return;
            }
            let feed = LAYOUT.feed_ratio[scale];
            self.push_counters[scale] += 1;
            if usize::from(self.push_counters[scale]) < feed {
                return;
            }
            self.push_counters[scale] = 0;
            value = self.compress(scale, offset + data_size, feed);
            if scale + 1 == LAST {
                // `PushToInfiniteScale`: the unbounded scale takes the compressed point at its front.
                self.infinite.insert(0, value);
                self.note_pushed(&value, LAST);
            }
        }
    }

    /// `CompressFunction`: the block of `n` points that starts `start` points into the scale's ring: the
    /// average of the present ones, positions and radius weighted by fertility.
    fn compress(&self, scale: usize, start: usize, n: usize) -> ZoneDataPoint {
        let mut present = 0_u32;
        let mut fertility = 0.0_f32;
        let mut x = 0.0_f32;
        let mut y = 0.0_f32;
        let mut radius = 0.0_f32;
        let mut biomass = 0.0_f32;
        for offset in 0..n {
            let point =
                self.data[LAYOUT.scale_offset[scale] + (start + offset) % LAYOUT.scale_size[scale]];
            if point.present() {
                present += 1;
                fertility += point.fertility;
                x += f32::from(point.pos_x) * point.fertility;
                y += f32::from(point.pos_y) * point.fertility;
                radius += f32::from(point.radius) * point.fertility;
                biomass += point.biomass;
            }
        }
        if present < 1 {
            return ZoneDataPoint::default();
        }
        ZoneDataPoint {
            pos_x: (x / fertility) as i16,
            pos_y: (y / fertility) as i16,
            radius: (radius / fertility) as u16,
            fertility: fertility / present as f32,
            biomass: biomass / present as f32,
        }
    }

    /// `NewPointPushedToScale`: moves the presence marks as a point enters a scale.
    fn note_pushed(&mut self, point: &ZoneDataPoint, scale: usize) {
        if point.present() {
            if scale > usize::from(self.scale_of_apparition) {
                self.scale_of_apparition = scale as u8;
                self.index_of_apparition = -1;
            }
            if scale <= usize::from(self.scale_of_disappearance) && self.index_of_disappearance >= 0
            {
                self.scale_of_disappearance = scale as u8;
                self.index_of_disappearance = -1;
                for j in
                    usize::from(self.scale_of_disappearance)..=usize::from(self.scale_of_apparition)
                {
                    self.n_present_at_scale[j] = if j == usize::from(self.scale_of_apparition) {
                        (i32::from(self.index_of_apparition) + 1) as u16
                    } else {
                        self.data_size_at_scale(j) as u16
                    };
                }
            }
        } else {
            if scale > usize::from(self.scale_of_disappearance)
                && i32::from(self.index_of_disappearance)
                    >= self.data_size_at_scale(usize::from(self.scale_of_disappearance)) as i32 - 1
            {
                self.scale_of_disappearance = scale as u8;
                self.index_of_disappearance = -1;
            }
            if scale == usize::from(self.scale_of_disappearance)
                && i32::from(self.index_of_disappearance)
                    < self.data_size_at_scale(scale) as i32 - 1
            {
                self.index_of_disappearance += 1;
                self.n_present_at_scale[scale] = self.n_present_at_scale[scale].wrapping_sub(1);
            }
        }
        if scale == usize::from(self.scale_of_apparition)
            && i32::from(self.index_of_apparition) < self.data_size_at_scale(scale) as i32 - 1
        {
            self.index_of_apparition += 1;
            self.n_present_at_scale[scale] = self.n_present_at_scale[scale].wrapping_add(1);
        }
    }

    /// The points in the order the game's loops visit them (and so `data.bin` stores them): the present
    /// points of each scale, then the accumulators that wait after its data.
    fn points_to_save(&self) -> Vec<ZoneDataPoint> {
        let mut points = Vec::with_capacity(self.n_points_to_save());
        for scale in 0..=usize::from(self.scale_of_apparition) {
            let first = if scale == usize::from(self.scale_of_disappearance) {
                i32::from(self.index_of_disappearance) + 1
            } else {
                0
            };
            let last = if scale == usize::from(self.scale_of_apparition) {
                i32::from(self.index_of_apparition)
            } else {
                self.data_size_at_scale(scale) as i32 - 1
            };
            if scale >= usize::from(self.scale_of_disappearance) && first >= 0 && last >= 0 {
                for index in first..=last {
                    points.push(self.point(scale, index as usize));
                }
            }
            if scale == LAST {
                break;
            }
            let end = LAYOUT.data_size[scale] + usize::from(self.push_counters[scale]);
            for index in LAYOUT.data_size[scale]..end {
                points.push(self.point(scale, index));
            }
        }
        points
    }

    /// `BytesSpace`.
    #[must_use]
    pub fn bytes_space(&self) -> usize {
        6 + 3 * (usize::from(self.scale_of_apparition) + 1)
            + self.n_points_to_save() * ZoneDataPoint::BYTE_SIZE
    }

    /// `SaveStateBin`: the log as the game writes it into `data.bin`.
    #[must_use]
    pub fn save_state_bin(&self) -> Vec<u8> {
        let mut bytes = vec![0_u8; self.bytes_space()];
        let n = self.n_points_to_save();
        let scales = self.scale_of_apparition + 1;
        bytes[0..2].copy_from_slice(&self.index_of_apparition.to_le_bytes());
        bytes[2..4].copy_from_slice(&self.index_of_disappearance.to_le_bytes());
        bytes[4] = self.scale_of_disappearance;
        bytes[5] = scales;
        if n < 1 {
            return bytes;
        }
        let scales = usize::from(scales);
        let mut offset = 6;
        for scale in 0..scales {
            bytes[offset + 2 * scale..offset + 2 * scale + 2]
                .copy_from_slice(&self.n_present_at_scale[scale].to_le_bytes());
        }
        for scale in 0..scales {
            bytes[offset + 2 * scales + scale] = self.push_counters[scale];
        }
        offset += 3 * scales;
        // The columns: `short[2n]` positions, `ushort[n]` radii, `float[n]` fertilities, `float[n]` biomasses.
        for (k, point) in self.points_to_save().iter().take(n).enumerate() {
            bytes[offset + 4 * k..offset + 4 * k + 2].copy_from_slice(&point.pos_x.to_le_bytes());
            bytes[offset + 4 * k + 2..offset + 4 * k + 4]
                .copy_from_slice(&point.pos_y.to_le_bytes());
            let at = offset + 4 * n + 2 * k;
            bytes[at..at + 2].copy_from_slice(&point.radius.to_le_bytes());
            let at = offset + 6 * n + 4 * k;
            bytes[at..at + 4].copy_from_slice(&point.fertility.to_le_bytes());
            let at = offset + 10 * n + 4 * k;
            bytes[at..at + 4].copy_from_slice(&point.biomass.to_le_bytes());
        }
        bytes
    }

    /// `LoadStateBin`: reads what [`ZoneLog::save_state_bin`] wrote.
    pub fn load_state_bin(bytes: &[u8]) -> Result<Self> {
        let mut log = Self::new();
        let short = |at: usize| -> Result<i16> {
            bytes
                .get(at..at + 2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]))
                .ok_or_else(|| Error::invalid_data("zone log: truncated header"))
        };
        log.index_of_apparition = short(0)?;
        log.index_of_disappearance = short(2)?;
        let (Some(&scale_of_disappearance), Some(&scales)) = (bytes.get(4), bytes.get(5)) else {
            return Err(Error::invalid_data("zone log: truncated header"));
        };
        log.scale_of_disappearance = scale_of_disappearance;
        if scales < 1 {
            log.scale_of_apparition = 0;
            return Ok(log);
        }
        let scales = usize::from(scales);
        if scales > N_SCALES || usize::from(scale_of_disappearance) >= N_SCALES {
            return Err(Error::invalid_data(
                "zone log: more scales than the config has",
            ));
        }
        log.scale_of_apparition = (scales - 1) as u8;
        let mut offset = 6;
        if bytes.len() < offset + 3 * scales {
            return Err(Error::invalid_data("zone log: truncated scale table"));
        }
        for scale in 0..scales {
            let at = offset + 2 * scale;
            log.n_present_at_scale[scale] = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
            log.push_counters[scale] = bytes[offset + 2 * scales + scale];
        }
        offset += 3 * scales;
        let n = log.n_points_to_save();
        if n < 1 {
            return Ok(log);
        }
        if bytes.len() < offset + n * ZoneDataPoint::BYTE_SIZE {
            return Err(Error::invalid_data("zone log: truncated points"));
        }
        if log.n_present_at_scale[LAST] > 0 {
            let missing = i32::from(log.index_of_apparition) + 1;
            for _ in 0..missing.max(0) {
                log.infinite.push(ZoneDataPoint::default());
            }
        }
        let read_u16 = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let read_i16 = |at: usize| i16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let read_f32 = |at: usize| {
            f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let point_at = |k: usize| ZoneDataPoint {
            pos_x: read_i16(offset + 4 * k),
            pos_y: read_i16(offset + 4 * k + 2),
            radius: read_u16(offset + 4 * n + 2 * k),
            fertility: read_f32(offset + 6 * n + 4 * k),
            biomass: read_f32(offset + 10 * n + 4 * k),
        };
        let mut k = 0;
        for scale in 0..=usize::from(log.scale_of_apparition) {
            let first = if scale == usize::from(log.scale_of_disappearance) {
                i32::from(log.index_of_disappearance) + 1
            } else {
                0
            };
            let last = if scale == usize::from(log.scale_of_apparition) {
                i32::from(log.index_of_apparition)
            } else {
                log.data_size_at_scale(scale) as i32 - 1
            };
            if scale >= usize::from(log.scale_of_disappearance) && first >= 0 && last >= 0 {
                for index in first..=last {
                    if k >= n {
                        return Err(Error::invalid_data(
                            "zone log: more present points than saved",
                        ));
                    }
                    if !log.set_point(scale, index as usize, point_at(k)) {
                        return Err(Error::invalid_data(
                            "zone log: a point past the unbounded scale",
                        ));
                    }
                    k += 1;
                }
            }
            if scale == LAST {
                break;
            }
            let end = LAYOUT.data_size[scale] + usize::from(log.push_counters[scale]);
            for index in LAYOUT.data_size[scale]..end {
                if k >= n {
                    return Err(Error::invalid_data(
                        "zone log: more accumulators than saved",
                    ));
                }
                log.set_point(scale, index, point_at(k));
                k += 1;
            }
        }
        for scale in 0..=usize::from(log.scale_of_apparition) {
            if scale == LAST {
                break;
            }
            log.n_data_at_scale[scale] = if scale == usize::from(log.scale_of_apparition) {
                (i32::from(log.index_of_apparition) + 1) as u8
            } else {
                LAYOUT.data_size[scale] as u8
            };
        }
        Ok(log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(step: i16) -> ZoneDataPoint {
        ZoneDataPoint {
            pos_x: 1000 * step,
            pos_y: -500 * step,
            radius: 4000,
            fertility: 2.0 + f32::from(step),
            biomass: 10.0 * f32::from(step),
        }
    }

    fn gone() -> ZoneDataPoint {
        ZoneDataPoint::default()
    }

    #[test]
    fn a_point_is_made_from_a_zones_relative_numbers() {
        let made = ZoneDataPoint::new([0.25, -2.0], 0.15, 3.5, 40.0);
        // The position is kept within +-0.9999 and scaled by 32767, truncated.
        assert_eq!(made.pos_x, 8191);
        assert_eq!(made.pos_y, -32763);
        assert_eq!(made.radius, 4915);
        assert_eq!((made.fertility, made.biomass), (3.5, 40.0));
        assert!(made.present());
        assert!((made.relative_pos()[0] - 0.25).abs() < 1e-3);
        assert!((made.relative_radius() - 0.15).abs() < 1e-3);
        // No radius or no growth: not there.
        assert!(!ZoneDataPoint::new([0.0, 0.0], 0.0, 1.0, 0.0).present());
        assert!(!ZoneDataPoint::new([0.0, 0.0], 0.1, 0.0, 0.0).present());
    }

    #[test]
    fn a_new_log_is_empty() {
        let log = ZoneLog::new();
        assert_eq!(log.total_index_of_apparition(), -1);
        assert_eq!(log.total_index_of_disappearance(), -1);
        assert_eq!(log.n_points_present(), 0);
        assert!(!log.at(0).present());
    }

    #[test]
    fn the_newest_point_is_index_zero() {
        let mut log = ZoneLog::new();
        for step in 1..=3 {
            log.push(point(step));
        }
        assert_eq!(log.at(0), point(3));
        assert_eq!(log.at(1), point(2));
        assert_eq!(log.at(2), point(1));
        assert_eq!(log.total_index_of_apparition(), 2);
        assert_eq!(log.total_index_of_disappearance(), -1);
        assert_eq!(log.n_points_present(), 3);
    }

    #[test]
    fn a_zone_that_vanishes_gets_a_disappearance_index() {
        let mut log = ZoneLog::new();
        log.push(point(1));
        log.push(point(2));
        log.push(gone());
        log.push(gone());
        assert_eq!(
            log.total_index_of_disappearance(),
            1,
            "the newest present point is index 2"
        );
        assert_eq!(log.total_index_of_apparition(), 3);
        assert_eq!(log.n_points_present(), 2);
        log.push(point(3));
        assert_eq!(log.total_index_of_disappearance(), -1, "back again");
    }

    #[test]
    fn old_points_are_compressed_into_one_weighted_by_fertility() {
        let mut log = ZoneLog::new();
        // 12 points fill the first scale; the next two age out and (feed ratio 2) are averaged.
        let heavy = ZoneDataPoint {
            pos_x: 3000,
            pos_y: 0,
            radius: 6000,
            fertility: 3.0,
            biomass: 30.0,
        };
        let light = ZoneDataPoint {
            pos_x: 0,
            pos_y: 0,
            radius: 3000,
            fertility: 1.0,
            biomass: 10.0,
        };
        log.push(heavy);
        log.push(light);
        for step in 0..12 {
            log.push(point(1 + (step % 3)));
        }
        // The oldest two (heavy, light) went to scale 1 as one point.
        let compressed = log.point(1, 0);
        assert_eq!(compressed.pos_x, 2250, "(3000 * 3 + 0 * 1) / 4");
        assert_eq!(compressed.radius, 5250, "(6000 * 3 + 3000 * 1) / 4");
        assert_eq!(compressed.fertility, 2.0, "the mean of the present");
        assert_eq!(compressed.biomass, 20.0);
        assert_eq!(log.at(12), compressed);
    }

    #[test]
    fn a_block_without_a_present_point_compresses_to_an_absent_one() {
        let mut log = ZoneLog::new();
        log.push(gone());
        log.push(gone());
        for _ in 0..12 {
            log.push(point(1));
        }
        assert!(!log.point(1, 0).present());
    }

    #[test]
    fn the_log_round_trips_through_data_bin_unchanged() {
        let mut log = ZoneLog::new();
        for k in 0..200 {
            log.push(if k % 17 < 3 {
                gone()
            } else {
                point(1 + (k % 5) as i16)
            });
        }
        let bytes = log.save_state_bin();
        assert_eq!(bytes.len(), log.bytes_space());
        let loaded = ZoneLog::load_state_bin(&bytes).expect("loads");
        assert_eq!(
            loaded.total_index_of_apparition(),
            log.total_index_of_apparition()
        );
        assert_eq!(
            loaded.total_index_of_disappearance(),
            log.total_index_of_disappearance()
        );
        assert_eq!(loaded.n_points_present(), log.n_points_present());
        for index in 0..log.total_index_of_apparition().max(0) as usize {
            assert_eq!(loaded.at(index), log.at(index), "point {index}");
        }
        assert_eq!(loaded.save_state_bin(), bytes, "written again as it was");
        // An empty log is its header.
        let empty = ZoneLog::new().save_state_bin();
        assert_eq!(empty.len(), 9);
        assert_eq!(
            ZoneLog::load_state_bin(&empty).expect("loads"),
            ZoneLog::new()
        );
    }

    #[test]
    fn old_points_are_kept_as_fewer_and_fewer_compressed_ones() {
        let mut log = ZoneLog::new();
        for k in 0..2000 {
            log.push(point(1 + (k % 4) as i16));
        }
        // 12 points at scale 0, 12 at scale 1 (every 2nd point), 18 (every 6th), 36 (every 12th), 32 (every
        // 36th) and the 3 the last scale has so far (every 72nd): 2000 logs are 113 points.
        assert_eq!(log.n_points_present(), 113);
        assert_eq!(log.total_index_of_apparition(), 112);
    }

    #[test]
    fn the_unbounded_scale_takes_the_oldest_points_and_is_saved() {
        // The first point reaches the unbounded scale after about a hundred thousand logs.
        let mut log = ZoneLog::new();
        for k in 0..300_000 {
            log.push(point(1 + (k % 3) as i16));
        }
        assert!(!log.infinite.is_empty());
        let finite = LAYOUT.data_sum[LAST - 1];
        assert!(log.total_index_of_apparition() > finite as i32);
        assert!(
            log.at(finite).present(),
            "the first unbounded point is read through the global index"
        );
        assert_eq!(log.at(finite), log.infinite[0]);
        let bytes = log.save_state_bin();
        let loaded = ZoneLog::load_state_bin(&bytes).expect("loads");
        assert_eq!(loaded.infinite, log.infinite);
        assert_eq!(loaded.save_state_bin(), bytes);
    }

    #[test]
    fn truncated_data_is_refused() {
        let mut log = ZoneLog::new();
        for k in 0..30 {
            log.push(point(1 + (k % 3) as i16));
        }
        let bytes = log.save_state_bin();
        assert!(ZoneLog::load_state_bin(&bytes[..bytes.len() - 5]).is_err());
        assert!(ZoneLog::load_state_bin(&bytes[..3]).is_err());
    }
}
