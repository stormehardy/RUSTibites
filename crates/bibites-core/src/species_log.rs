//! The multi-resolution history of one species (`Here/Utility/LogLikeArraySerial.cs`,
//! `LogLikePresenceArray.cs`, `LogLikeSpeciesDataArray.cs`, `SpeciesDataPoint.cs`).
//!
//! The game logs every species every 5 simulated minutes ([`ACQUISITION_MINUTES`]). The points live in a
//! log-like array: scale 0 holds the newest 12 points, and each scale hands the points that age out of it, in
//! blocks of its feed ratio, compressed into one point to the next, coarser scale. The last scale is unbounded.
//! Point index 0 is the newest one; the scales are numbered after each other, so the global index of a point
//! is the number of points before it (`totalIndexOf…`).
//!
//! What the lineage tree reads is where the species' presence starts and ends in that index space: the
//! apparition (the oldest present point) and the disappearance (the newest point that is gone; `-1` while the
//! species is alive). The count of present points (`nPointsPresent`) is what pruning compares with the
//! prune point limit.
//!
//! Saved in `data.bin` as the game does (`SaveStateBin` / `LoadStateBin`): positions are quantised against the
//! simulation size, so a log re-saved with a different size changes by rounding.

use crate::datalog::LogLikeFormat;
use crate::error::{Error, Result};

/// `DataLogger.SerialSpeciesConfig.acquisitionFactor` (the acquisition unit is minutes): the species are
/// logged every this many simulated minutes.
pub const ACQUISITION_MINUTES: u32 = 5;

/// `DataLogger.SerialSpeciesConfig.formats`; the last scale is unbounded (`LogLikeFormat.Infinite()`).
pub const SPECIES_FORMATS: [LogLikeFormat; 11] = [
    LogLikeFormat::new(12, 2),
    LogLikeFormat::new(12, 3),
    LogLikeFormat::new(18, 2),
    LogLikeFormat::new(36, 3),
    LogLikeFormat::new(32, 2),
    LogLikeFormat::new(36, 2),
    LogLikeFormat::new(30, 2),
    LogLikeFormat::new(30, 2),
    LogLikeFormat::new(30, 5),
    LogLikeFormat::new(24, 3),
    LogLikeFormat::new(0, 0),
];

pub(crate) const N_SCALES: usize = SPECIES_FORMATS.len();
pub(crate) const LAST: usize = N_SCALES - 1;

/// The sizes the scales add up to (`LogLikeArray`'s constructor with `needsAccumulation`: every finite scale
/// keeps `feedRatio` more points than it shows, the ones waiting to be compressed).
pub(crate) struct Layout {
    /// `scaleSizes`: data plus accumulators.
    pub(crate) scale_size: [usize; N_SCALES],
    /// `scaleOffsets`: where a scale starts in the ring buffer storage.
    pub(crate) scale_offset: [usize; N_SCALES],
    /// `dataSizes`.
    pub(crate) data_size: [usize; N_SCALES],
    /// `scaleDataSum`: the points of the scales up to and including this one.
    pub(crate) data_sum: [usize; N_SCALES],
    /// `feedRatios`.
    pub(crate) feed_ratio: [usize; N_SCALES],
    /// The points of every finite scale together.
    pub(crate) total: usize,
}

const fn layout() -> Layout {
    let mut scale_size = [0; N_SCALES];
    let mut scale_offset = [0; N_SCALES];
    let mut data_size = [0; N_SCALES];
    let mut data_sum = [0; N_SCALES];
    let mut feed_ratio = [0; N_SCALES];
    let mut total = 0;
    let mut i = 0;
    while i < N_SCALES {
        let format = SPECIES_FORMATS[i];
        data_size[i] = format.size;
        scale_size[i] = format.size + format.feed_ratio;
        feed_ratio[i] = format.feed_ratio;
        data_sum[i] = format.size;
        total += scale_size[i];
        if i > 0 {
            scale_offset[i] = scale_offset[i - 1] + scale_size[i - 1];
            data_sum[i] += data_sum[i - 1];
        }
        i += 1;
    }
    Layout {
        scale_size,
        scale_offset,
        data_size,
        data_sum,
        feed_ratio,
        total,
    }
}

pub(crate) static LAYOUT: Layout = layout();

/// One logged sample of a species (`SpeciesDataPoint`): how many members it had, their energy, and where they
/// were (the energy weighted mean position, the standard deviation per axis and the correlation of the axes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpeciesDataPoint {
    /// Bibites plus eggs of the species (`ushort`).
    pub count: u16,
    /// Their summed energy.
    pub energy: f32,
    /// The energy weighted mean position.
    pub avg_pos: [f32; 2],
    /// The standard deviation of the position per axis.
    pub pos_std_dev: [f32; 2],
    /// The correlation of x and y.
    pub pos_cor: f32,
}

/// One member of a species at a log tick: a bibite (its `totalEnergy`) or an egg (its `energy`) and where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeciesMember {
    /// The energy the point weights this member with.
    pub energy: f32,
    /// The world position.
    pub pos: [f32; 2],
}

impl SpeciesDataPoint {
    /// Bytes of a point in the file (`SpeciesDataPoint.byteSize`).
    pub const BYTE_SIZE: usize = 16;

    /// A sample that only knows how many members there were and their energy, without positions (what a
    /// registry without a world to ask logs, and what an old checkpoint's samples become).
    #[must_use]
    pub fn from_counts(count: u32, energy: f32) -> Self {
        if count < 1 {
            return Self::default();
        }
        Self {
            count: count.min(u32::from(u16::MAX)) as u16,
            energy,
            avg_pos: [0.0, 0.0],
            pos_std_dev: if count < 2 { [1.0, 1.0] } else { [10.0, 10.0] },
            pos_cor: 0.0,
        }
    }

    /// `alive`: the species had members at the sample.
    #[must_use]
    pub fn alive(&self) -> bool {
        self.count > 0
    }

    /// `espXY`: the mean of x * y.
    #[must_use]
    pub fn esp_xy(&self) -> f32 {
        self.pos_cor * self.pos_std_dev[0] * self.pos_std_dev[1] + self.avg_pos[0] * self.avg_pos[1]
    }

    /// `StdDevWithPos`: the variance the point has around another position.
    #[must_use]
    pub fn std_dev_with_pos(&self, pos: [f32; 2]) -> [f32; 2] {
        let dx = self.avg_pos[0] - pos[0];
        let dy = self.avg_pos[1] - pos[1];
        [
            self.pos_std_dev[0] * self.pos_std_dev[0] + dx * dx,
            self.pos_std_dev[1] * self.pos_std_dev[1] + dy * dy,
        ]
    }

    /// `new SpeciesDataPoint(species)`: the sample of a species whose bibites then eggs are given (the
    /// accumulations run in that order, as the game's loops do).
    #[must_use]
    pub fn from_members(bibites: &[SpeciesMember], eggs: &[SpeciesMember]) -> Self {
        let count = (bibites.len() + eggs.len()) as u16;
        if count < 1 {
            return Self::default();
        }
        let members = || bibites.iter().chain(eggs.iter());
        let mut energy = 0.0_f32;
        let mut weighted = [0.0_f32; 2];
        let mut cross = 0.0_f32;
        for member in members() {
            energy += member.energy;
            weighted[0] += member.pos[0] * member.energy;
            weighted[1] += member.pos[1] * member.energy;
            cross += member.pos[0] * member.pos[1] * member.energy;
        }
        let avg_pos = [weighted[0] / energy, weighted[1] / energy];
        let mut spread = [0.0_f32; 2];
        for member in members() {
            let dx = member.pos[0] - avg_pos[0];
            let dy = member.pos[1] - avg_pos[1];
            spread[0] += dx * dx * member.energy;
            spread[1] += dy * dy * member.energy;
        }
        let mut pos_std_dev = [
            (spread[0] / energy).sqrt().max(10.0),
            (spread[1] / energy).sqrt().max(10.0),
        ];
        let mut pos_cor =
            (cross / energy - avg_pos[0] * avg_pos[1]) / (pos_std_dev[0] * pos_std_dev[1]);
        if count < 2 {
            pos_cor = 0.0;
            pos_std_dev = [1.0, 1.0];
        }
        Self {
            count,
            energy,
            avg_pos,
            pos_std_dev,
            pos_cor,
        }
    }
}

/// The history of one species: a [`SpeciesDataPoint`] log-like array that knows where the species is present.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesLog {
    /// `LogLikeSpeciesDataArray.absolutePreserve`: a template species keeps a compressed point alive as soon as
    /// one of its points was, any other needs half of them.
    absolute_preserve: bool,
    /// The ring buffers of the finite scales, one after the other (`data`).
    data: Vec<SpeciesDataPoint>,
    /// The unbounded last scale, newest first (`infiniteData`).
    infinite: Vec<SpeciesDataPoint>,
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

impl Default for SpeciesLog {
    /// A log of a species that did not come from a template (`new LogLikeSpeciesDataArray(false)`).
    fn default() -> Self {
        Self::new(false)
    }
}

/// Bytes of a point in [`SpeciesLog::encode_exact`]: the count, the energy, then five floats.
const EXACT_POINT_BYTES: usize = 2 + 4 + 5 * 4;

impl SpeciesLog {
    /// `new LogLikeSpeciesDataArray(isTemplate)`.
    #[must_use]
    pub fn new(is_template: bool) -> Self {
        Self {
            absolute_preserve: is_template,
            data: vec![SpeciesDataPoint::default(); LAYOUT.total],
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

    /// `totalIndexOfApparition`: the global index of the oldest present point.
    #[must_use]
    pub fn total_index_of_apparition(&self) -> i32 {
        let before = match self.scale_of_apparition {
            0 => 0,
            scale => LAYOUT.data_sum[usize::from(scale) - 1] as i32,
        };
        i32::from(self.index_of_apparition) + before
    }

    /// `totalIndexOfDisappearance`: the global index of the newest point that is gone, `-1` for a species that
    /// is present in the newest point.
    #[must_use]
    pub fn total_index_of_disappearance(&self) -> i32 {
        let before = match self.scale_of_disappearance {
            0 => 0,
            scale => LAYOUT.data_sum[usize::from(scale) - 1] as i32,
        };
        i32::from(self.index_of_disappearance) + before
    }

    /// The species is present in the newest point (`totalIndexOfDisappearance < 0`).
    #[must_use]
    pub fn alive(&self) -> bool {
        self.total_index_of_disappearance() < 0
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
    #[must_use]
    pub fn data_size_at_scale(&self, scale: usize) -> usize {
        if scale == LAST {
            self.infinite.len().max(LAYOUT.data_size[LAST - 1])
        } else {
            LAYOUT.data_size[scale]
        }
    }

    /// The point at `index` of `scale` (`this[scale, i]`); a default (not alive) point out of range.
    #[must_use]
    pub fn point(&self, scale: usize, index: usize) -> SpeciesDataPoint {
        if scale >= N_SCALES {
            return SpeciesDataPoint::default();
        }
        if scale == LAST {
            return self.infinite.get(index).copied().unwrap_or_default();
        }
        self.data[self.ring_index(scale, index)]
    }

    /// The point at the global index `index` (`this[i]`: 0 is the newest point, the scales follow each other),
    /// as the Position tab's time slider reads it; a default (not alive) point past the end.
    #[must_use]
    pub fn at(&self, index: usize) -> SpeciesDataPoint {
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
    fn set_point(&mut self, scale: usize, index: usize, point: SpeciesDataPoint) -> bool {
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
    pub fn push(&mut self, point: SpeciesDataPoint) {
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
            if scale >= LAST {
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

    /// `CompressFunction`: the block of `n` points that starts `start` points into the scale's ring.
    fn compress(&self, scale: usize, start: usize, n: usize) -> SpeciesDataPoint {
        // No feed ratio is above 5.
        let mut block = [SpeciesDataPoint::default(); 8];
        for (o, slot) in block.iter_mut().enumerate().take(n) {
            *slot = self.data[LAYOUT.scale_offset[scale] + (start + o) % LAYOUT.scale_size[scale]];
        }
        let block = &block[..n.min(8)];
        let mut result = SpeciesDataPoint::default();
        let mut present = 0_i32;
        let mut counts = 0_i32;
        let mut energy = 0.0_f32;
        let mut weighted = [0.0_f32; 2];
        let mut cross = 0.0_f32;
        for point in block {
            if point.alive() {
                present += 1;
                counts += i32::from(point.count);
                energy += point.energy;
                weighted[0] += point.avg_pos[0] * point.energy;
                weighted[1] += point.avg_pos[1] * point.energy;
                cross += point.esp_xy() * point.energy;
            }
        }
        let needed = (n as i32 + 1) / 2;
        if (!self.absolute_preserve && present < needed) || (self.absolute_preserve && present < 1)
        {
            return result;
        }
        result.count = (counts / present).max(1) as u16;
        result.energy = energy / present as f32;
        result.avg_pos = [weighted[0] / energy, weighted[1] / energy];
        let mut variance = [0.0_f32; 2];
        for point in block {
            if point.alive() {
                let around = point.std_dev_with_pos(result.avg_pos);
                variance[0] += around[0] * point.energy;
                variance[1] += around[1] * point.energy;
            }
        }
        result.pos_std_dev = [(variance[0] / energy).sqrt(), (variance[1] / energy).sqrt()];
        result.pos_cor = (cross / energy - result.avg_pos[0] * result.avg_pos[1])
            / (result.pos_std_dev[0] * result.pos_std_dev[1]);
        result
    }

    /// `NewPointPushedToScale`: moves the presence marks as a point enters a scale.
    fn note_pushed(&mut self, point: &SpeciesDataPoint, scale: usize) {
        if point.alive() {
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

    /// `RefreshPresentsFromIndices`.
    fn refresh_presents_from_indices(&mut self) {
        for scale in
            usize::from(self.scale_of_disappearance)..=usize::from(self.scale_of_apparition)
        {
            if (scale as i32) < i32::from(self.scale_of_apparition) - 1 {
                self.n_data_at_scale[scale] = LAYOUT.data_size[scale] as u8;
            }
            let size = self.data_size_at_scale(scale) as i32;
            let first = if scale == usize::from(self.scale_of_disappearance) {
                i32::from(self.index_of_disappearance) + 1
            } else {
                0
            };
            let last = if scale == usize::from(self.scale_of_apparition) {
                i32::from(self.index_of_apparition)
            } else {
                size - 1
            };
            self.n_present_at_scale[scale] = (last - first + 1).clamp(0, size) as u16;
            if scale == usize::from(self.scale_of_apparition) && scale < LAST {
                self.n_data_at_scale[scale] = self.n_present_at_scale[scale] as u8;
            }
        }
    }

    /// `IncorporateSpeciesData`: a pruned species' history is merged into this one (the survivor), adding the
    /// counts and energies of the points they share and weighting the positions with the energies.
    pub fn incorporate(&mut self, other: &Self) {
        for scale in
            usize::from(other.scale_of_disappearance)..=usize::from(other.scale_of_apparition)
        {
            let size = other.data_size_at_scale(scale) as i32;
            let first = if scale == usize::from(other.scale_of_disappearance) {
                i32::from(other.index_of_disappearance) + 1
            } else {
                0
            };
            let mut last = if scale == usize::from(other.scale_of_apparition) {
                i32::from(other.index_of_apparition)
            } else {
                size - 1
            };
            if first < 0 || last < 0 {
                continue;
            }
            if last == size - 1 {
                last += LAYOUT.feed_ratio[scale] as i32;
            }
            for j in first..=last {
                let mut value = self.point(scale, j as usize);
                let theirs = other.point(scale, j as usize);
                if !theirs.alive() {
                    continue;
                }
                let was_alive = value.alive();
                let mine_energy = value.energy;
                let their_energy = theirs.energy;
                let total = mine_energy + their_energy;
                let avg = [
                    (value.avg_pos[0] * mine_energy + theirs.avg_pos[0] * their_energy) / total,
                    (value.avg_pos[1] * mine_energy + theirs.avg_pos[1] * their_energy) / total,
                ];
                let mine_var = value.std_dev_with_pos(avg);
                let their_var = theirs.std_dev_with_pos(avg);
                let variance = [
                    (mine_var[0] * mine_energy + their_var[0] * their_energy) / total,
                    (mine_var[1] * mine_energy + their_var[1] * their_energy) / total,
                ];
                let esp = (value.esp_xy() * mine_energy + theirs.esp_xy() * their_energy) / total;
                value.avg_pos = avg;
                value.pos_std_dev = [variance[0].sqrt(), variance[1].sqrt()];
                value.pos_cor =
                    (esp - avg[0] * avg[1]) / (value.pos_std_dev[0] * value.pos_std_dev[1]);
                value.count = value.count.wrapping_add(theirs.count);
                value.energy = total;
                self.set_point(scale, j as usize, value);
                if !was_alive && j < size {
                    if scale > usize::from(self.scale_of_apparition)
                        || (scale == usize::from(self.scale_of_apparition)
                            && j > i32::from(self.index_of_apparition))
                    {
                        self.scale_of_apparition = scale as u8;
                        self.index_of_apparition = j as i16;
                    }
                    if scale < usize::from(self.scale_of_disappearance)
                        || (scale == usize::from(self.scale_of_disappearance)
                            && j <= i32::from(self.index_of_disappearance))
                    {
                        self.scale_of_disappearance = scale as u8;
                        self.index_of_disappearance = (j - 1) as i16;
                    }
                }
            }
        }
        self.refresh_presents_from_indices();
    }

    /// The present points as `(scale, index)` pairs from the newest to the oldest (the loops of
    /// `SaveStateBin`), without the accumulators.
    #[must_use]
    pub fn present_points(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
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
                    out.push((scale, index as usize));
                }
            }
        }
        out
    }

    /// A lossless encoding of every field, the ring buffers included, for checkpoints (the game's own file
    /// format quantises the positions).
    #[must_use]
    pub fn encode_exact(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(64 + (self.data.len() + self.infinite.len()) * EXACT_POINT_BYTES);
        out.push(u8::from(self.absolute_preserve));
        for offset in &self.index_offsets {
            out.extend_from_slice(&(*offset as u16).to_le_bytes());
        }
        out.extend_from_slice(&self.push_counters);
        out.extend_from_slice(&self.n_data_at_scale);
        for present in &self.n_present_at_scale {
            out.extend_from_slice(&present.to_le_bytes());
        }
        out.push(self.scale_of_apparition);
        out.extend_from_slice(&self.index_of_apparition.to_le_bytes());
        out.push(self.scale_of_disappearance);
        out.extend_from_slice(&self.index_of_disappearance.to_le_bytes());
        out.extend_from_slice(&(self.infinite.len() as u32).to_le_bytes());
        for point in self.data.iter().chain(&self.infinite) {
            out.extend_from_slice(&point.count.to_le_bytes());
            out.extend_from_slice(&point.energy.to_le_bytes());
            for value in [
                point.avg_pos[0],
                point.avg_pos[1],
                point.pos_std_dev[0],
                point.pos_std_dev[1],
                point.pos_cor,
            ] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }

    /// Reads what [`SpeciesLog::encode_exact`] wrote.
    pub fn decode_exact(bytes: &[u8]) -> Result<Self> {
        let fixed = 1 + 2 * N_SCALES + 2 * N_SCALES + 2 * N_SCALES + 3 + 3 + 4;
        if bytes.len() < fixed {
            return Err(Error::invalid_data("species log: truncated checkpoint"));
        }
        let mut at = 0;
        let mut log = Self::new(bytes[at] != 0);
        at += 1;
        for offset in &mut log.index_offsets {
            *offset = usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
            at += 2;
        }
        for scale in 0..N_SCALES {
            if log.index_offsets[scale] >= LAYOUT.scale_size[scale].max(1) {
                return Err(Error::invalid_data("species log: ring offset out of range"));
            }
        }
        log.push_counters.copy_from_slice(&bytes[at..at + N_SCALES]);
        at += N_SCALES;
        log.n_data_at_scale
            .copy_from_slice(&bytes[at..at + N_SCALES]);
        at += N_SCALES;
        for present in &mut log.n_present_at_scale {
            *present = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
            at += 2;
        }
        log.scale_of_apparition = bytes[at];
        log.index_of_apparition = i16::from_le_bytes([bytes[at + 1], bytes[at + 2]]);
        log.scale_of_disappearance = bytes[at + 3];
        log.index_of_disappearance = i16::from_le_bytes([bytes[at + 4], bytes[at + 5]]);
        at += 6;
        if usize::from(log.scale_of_apparition) >= N_SCALES
            || usize::from(log.scale_of_disappearance) >= N_SCALES
        {
            return Err(Error::invalid_data("species log: scale out of range"));
        }
        let infinite =
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        at += 4;
        let points = LAYOUT.total + infinite;
        if bytes.len() != at + points * EXACT_POINT_BYTES {
            return Err(Error::invalid_data(
                "species log: checkpoint size does not match its points",
            ));
        }
        let f32_at = |at: usize| {
            f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let read_point = |at: &mut usize| {
            let point = SpeciesDataPoint {
                count: u16::from_le_bytes([bytes[*at], bytes[*at + 1]]),
                energy: f32_at(*at + 2),
                avg_pos: [f32_at(*at + 6), f32_at(*at + 10)],
                pos_std_dev: [f32_at(*at + 14), f32_at(*at + 18)],
                pos_cor: f32_at(*at + 22),
            };
            *at += EXACT_POINT_BYTES;
            point
        };
        for slot in 0..LAYOUT.total {
            log.data[slot] = read_point(&mut at);
        }
        for _ in 0..infinite {
            let point = read_point(&mut at);
            log.infinite.push(point);
        }
        Ok(log)
    }

    /// `BytesSpace`.
    #[must_use]
    pub fn bytes_space(&self) -> usize {
        6 + 3 * (usize::from(self.scale_of_apparition) + 1)
            + self.n_points_to_save() * SpeciesDataPoint::BYTE_SIZE
    }

    /// `SaveStateBin`: the log as the game writes it into `data.bin`. `simulation_size` is the
    /// `SimulationSize` setting positions are quantised against.
    #[must_use]
    pub fn save_state_bin(&self, simulation_size: f32) -> Vec<u8> {
        let mut bytes = vec![0_u8; self.bytes_space()];
        let n_points = self.n_points_to_save();
        let scales = self.scale_of_apparition + 1;
        bytes[0..2].copy_from_slice(&self.index_of_apparition.to_le_bytes());
        bytes[2..4].copy_from_slice(&self.index_of_disappearance.to_le_bytes());
        bytes[4] = self.scale_of_disappearance;
        bytes[5] = scales;
        if n_points < 1 {
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
        // The points in the order the game's loops visit them: the present points of a scale, then the
        // accumulators that wait after its data.
        let mut points = Vec::with_capacity(n_points);
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
        let factor = size_factor(simulation_size);
        let n = n_points;
        let quantise = |value: f32| (value as i32) as i16;
        for (k, point) in points.iter().take(n).enumerate() {
            bytes[offset + 2 * k..offset + 2 * k + 2].copy_from_slice(&point.count.to_le_bytes());
            bytes[offset + 2 * n + 4 * k..offset + 2 * n + 4 * k + 4]
                .copy_from_slice(&point.energy.to_le_bytes());
            let positions = [
                quantise(point.avg_pos[0] * factor),
                quantise(point.avg_pos[1] * factor),
            ];
            let at = offset + 6 * n + 4 * k;
            bytes[at..at + 2].copy_from_slice(&positions[0].to_le_bytes());
            bytes[at + 2..at + 4].copy_from_slice(&positions[1].to_le_bytes());
            let spread = [
                ((point.pos_std_dev[0] * 2.0 * factor) as i32) as u16,
                ((point.pos_std_dev[1] * 2.0 * factor) as i32) as u16,
            ];
            let at = offset + 10 * n + 4 * k;
            bytes[at..at + 2].copy_from_slice(&spread[0].to_le_bytes());
            bytes[at + 2..at + 4].copy_from_slice(&spread[1].to_le_bytes());
            let cor = quantise(point.pos_cor * 32766.0);
            let at = offset + 14 * n + 2 * k;
            bytes[at..at + 2].copy_from_slice(&cor.to_le_bytes());
        }
        bytes
    }

    /// `LoadStateBin`: reads what [`SpeciesLog::save_state_bin`] wrote (`simulation_size` as for saving).
    pub fn load_state_bin(bytes: &[u8], simulation_size: f32, is_template: bool) -> Result<Self> {
        let mut log = Self::new(is_template);
        let short = |at: usize| -> Result<i16> {
            let slice = bytes.get(at..at + 2);
            slice
                .map(|s| i16::from_le_bytes([s[0], s[1]]))
                .ok_or_else(|| Error::invalid_data("species log: truncated header"))
        };
        log.index_of_apparition = short(0)?;
        log.index_of_disappearance = short(2)?;
        let (Some(&scale_of_disappearance), Some(&scales)) = (bytes.get(4), bytes.get(5)) else {
            return Err(Error::invalid_data("species log: truncated header"));
        };
        log.scale_of_disappearance = scale_of_disappearance;
        if scales < 1 {
            log.scale_of_apparition = 0;
            return Ok(log);
        }
        let scales = usize::from(scales);
        if scales > N_SCALES || usize::from(scale_of_disappearance) >= N_SCALES {
            return Err(Error::invalid_data(
                "species log: more scales than the config has",
            ));
        }
        log.scale_of_apparition = (scales - 1) as u8;
        let mut offset = 6;
        if bytes.len() < offset + 3 * scales {
            return Err(Error::invalid_data("species log: truncated scale table"));
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
        if bytes.len() < offset + n * SpeciesDataPoint::BYTE_SIZE {
            return Err(Error::invalid_data("species log: truncated points"));
        }
        if log.n_present_at_scale[LAST] > 0 {
            let missing = i32::from(log.index_of_apparition) + 1;
            for _ in 0..missing.max(0) {
                log.infinite.push(SpeciesDataPoint::default());
            }
        }
        let factor = size_factor(simulation_size);
        let read_u16 = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let read_i16 = |at: usize| i16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let point_at = |k: usize| -> SpeciesDataPoint {
            let count = read_u16(offset + 2 * k);
            let energy_at = offset + 2 * n + 4 * k;
            let energy = f32::from_le_bytes([
                bytes[energy_at],
                bytes[energy_at + 1],
                bytes[energy_at + 2],
                bytes[energy_at + 3],
            ]);
            let pos_at = offset + 6 * n + 4 * k;
            let spread_at = offset + 10 * n + 4 * k;
            SpeciesDataPoint {
                count,
                energy,
                avg_pos: [
                    f32::from(read_i16(pos_at)) / factor,
                    f32::from(read_i16(pos_at + 2)) / factor,
                ],
                pos_std_dev: [
                    f32::from(read_u16(spread_at)) / (2.0 * factor),
                    f32::from(read_u16(spread_at + 2)) / (2.0 * factor),
                ],
                pos_cor: f32::from(read_i16(offset + 14 * n + 2 * k)) / 32767.0,
            }
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
                            "species log: more present points than saved",
                        ));
                    }
                    if !log.set_point(scale, index as usize, point_at(k)) {
                        return Err(Error::invalid_data(
                            "species log: a point past the unbounded scale",
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
                        "species log: more accumulators than saved",
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

/// `32767 / SimulationSize / 2`: how many file units a world unit is.
fn size_factor(simulation_size: f32) -> f32 {
    32767.0 / simulation_size / 2.0
}

/// `TimeOfPointInSerialConfig`: how many minutes ago the global point index `point` was logged (`-1` is now).
#[must_use]
pub fn minutes_of_point(point: i32) -> i64 {
    let mut step = i64::from(ACQUISITION_MINUTES);
    let mut minutes = 0_i64;
    let mut remaining = i64::from(point) + 1;
    for (i, format) in SPECIES_FORMATS.iter().enumerate() {
        let size = if i == LAST {
            i64::from(i32::MAX)
        } else {
            format.size as i64
        };
        if remaining > size {
            minutes += step * size;
            remaining -= size;
            step *= format.feed_ratio as i64;
            continue;
        }
        minutes += step * remaining;
        break;
    }
    minutes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(count: u16) -> SpeciesDataPoint {
        SpeciesDataPoint {
            count,
            energy: f32::from(count) * 10.0,
            avg_pos: [100.0 * f32::from(count), -50.0],
            pos_std_dev: [20.0, 30.0],
            pos_cor: 0.25,
        }
    }

    /// Two files of the same log differ only by the one-unit rounding of the quantised positions (the game
    /// has it too: a position is read as `short / factor` and written as `(short)(value * factor)`).
    fn same_up_to_rounding(a: &[u8], b: &[u8], what: &str) {
        assert_eq!(a.len(), b.len(), "{what}");
        let header = 6 + 3 * usize::from(a[5]);
        if a.len() <= header {
            assert_eq!(a, b, "{what}");
            return;
        }
        let n = (a.len() - header) / SpeciesDataPoint::BYTE_SIZE;
        // The scale table, the counts and the energies are not quantised.
        let exact = header + 6 * n;
        assert_eq!(&a[..exact], &b[..exact], "{what}: table, counts, energies");
        for at in (exact..a.len()).step_by(2) {
            let x = i32::from(i16::from_le_bytes([a[at], a[at + 1]]));
            let y = i32::from(i16::from_le_bytes([b[at], b[at + 1]]));
            assert!(
                (x - y).abs() <= 1,
                "{what}: quantised word at {at}: {x} vs {y}"
            );
        }
    }

    fn alive_log(pushes: usize) -> SpeciesLog {
        let mut log = SpeciesLog::new(false);
        for k in 0..pushes {
            log.push(point(1 + (k % 5) as u16));
        }
        log
    }

    #[test]
    fn a_new_log_is_empty_and_alive() {
        let log = SpeciesLog::new(false);
        assert_eq!(log.total_index_of_apparition(), -1);
        assert_eq!(log.total_index_of_disappearance(), -1);
        assert_eq!(log.n_points_present(), 0);
        assert!(log.alive());
    }

    #[test]
    fn the_first_scale_fills_one_point_at_a_time() {
        let log = alive_log(5);
        assert_eq!(log.total_index_of_apparition(), 4);
        assert_eq!(log.total_index_of_disappearance(), -1);
        assert_eq!(log.n_points_present(), 5);
        // The newest point is index 0.
        assert_eq!(log.point(0, 0).count, 5);
        assert_eq!(log.point(0, 4).count, 1);
    }

    #[test]
    fn the_scales_hand_compressed_points_to_the_next() {
        // 12 points fill scale 0; the 13th waits as an accumulator; the 14th makes the block of two a point of scale 1.
        let log = alive_log(12);
        assert_eq!(log.total_index_of_apparition(), 11);
        let log = alive_log(13);
        assert_eq!(
            log.total_index_of_apparition(),
            11,
            "an accumulator is not a present point yet"
        );
        assert_eq!(log.n_points_present(), 12);
        let log = alive_log(14);
        assert_eq!(log.scale_of_apparition, 1);
        assert_eq!(log.total_index_of_apparition(), 12);
        assert_eq!(log.n_points_present(), 13);
        // The compressed point averages the counts of the block and sums energies per point.
        let compressed = log.point(1, 0);
        assert!(compressed.alive());
        assert!(compressed.pos_std_dev[0] > 0.0);
    }

    #[test]
    fn a_species_that_dies_moves_its_disappearance_up() {
        let mut log = alive_log(6);
        assert!(log.alive());
        log.push(SpeciesDataPoint::default());
        assert!(!log.alive());
        assert_eq!(log.total_index_of_disappearance(), 0);
        assert_eq!(log.total_index_of_apparition(), 6);
        assert_eq!(log.n_points_present(), 6);
        log.push(SpeciesDataPoint::default());
        log.push(SpeciesDataPoint::default());
        assert_eq!(log.total_index_of_disappearance(), 2);
        assert_eq!(log.total_index_of_apparition(), 8);
        // The present points are the six that lived, between the two marks.
        assert_eq!(
            log.total_index_of_apparition() - log.total_index_of_disappearance(),
            log.n_points_present() as i32
        );
    }

    #[test]
    fn a_species_that_returns_is_alive_again() {
        let mut log = alive_log(4);
        log.push(SpeciesDataPoint::default());
        log.push(SpeciesDataPoint::default());
        assert_eq!(log.total_index_of_disappearance(), 1);
        log.push(point(2));
        assert!(log.alive(), "alive in the newest point");
        assert_eq!(log.total_index_of_disappearance(), -1);
    }

    #[test]
    fn compression_needs_half_the_block_unless_it_is_a_template() {
        // Scale 1 feeds scale 2 in blocks of three. Its points go alive, dead, dead, alive, ... (each one is a
        // block of two pushes): one alive of three is under the half a plain species needs, and enough for a
        // template species.
        let mut plain = SpeciesLog::new(false);
        let mut template = SpeciesLog::new(true);
        for log in [&mut plain, &mut template] {
            for k in 0..600 {
                let alive = (k / 2) % 3 == 0;
                log.push(if alive {
                    point(2)
                } else {
                    SpeciesDataPoint::default()
                });
            }
        }
        let alive_at_scale_2 =
            |log: &SpeciesLog| (0..18).filter(|&i| log.point(2, i).alive()).count();
        assert_eq!(
            alive_at_scale_2(&plain),
            0,
            "one alive of three is not half"
        );
        assert!(
            alive_at_scale_2(&template) > 0,
            "a template keeps what one point lived"
        );
    }

    #[test]
    fn the_unbounded_scale_takes_the_oldest_points() {
        // The first point reaches the unbounded scale after about a hundred thousand logs (some years of
        // simulated time).
        let mut log = SpeciesLog::new(false);
        for k in 0..300_000 {
            log.push(point(1 + (k % 3) as u16));
        }
        assert_eq!(log.scale_of_apparition as usize, LAST);
        assert!(!log.infinite.is_empty());
        assert_eq!(log.total_index_of_disappearance(), -1);
        assert!(log.total_index_of_apparition() > LAYOUT.data_sum[LAST - 1] as i32);
    }

    #[test]
    fn the_global_index_walks_the_scales_from_the_newest_point() {
        let mut log = SpeciesLog::new(false);
        for k in 0..40 {
            log.push(point(1 + (k % 7) as u16));
        }
        // Scale 0 holds the newest 12, then scale 1 the next 12 (each the mean of two), and so on.
        for index in 0..12 {
            assert_eq!(log.at(index), log.point(0, index), "{index}");
        }
        for index in 0..12 {
            assert_eq!(log.at(12 + index), log.point(1, index), "{index}");
        }
        assert_eq!(log.at(24), log.point(2, 0));
        // Past the last point there is nothing.
        assert!(!log.at(500).alive());
        // The unbounded scale follows the finite ones.
        let mut long = SpeciesLog::new(false);
        for k in 0..300_000 {
            long.push(point(1 + (k % 3) as u16));
        }
        let finite = LAYOUT.data_sum[LAST - 1];
        assert_eq!(long.at(finite), long.infinite[0]);
        assert_eq!(long.at(finite + 1), long.infinite[1]);
    }

    #[test]
    fn time_of_a_point_follows_the_scales() {
        assert_eq!(minutes_of_point(-1), 0);
        assert_eq!(minutes_of_point(0), 5);
        assert_eq!(minutes_of_point(11), 60);
        // The first point of scale 1 is 10 minutes per step after the 12 of scale 0.
        assert_eq!(minutes_of_point(12), 60 + 10);
        assert_eq!(minutes_of_point(23), 60 + 120);
        // Scale 2 steps by 30 minutes.
        assert_eq!(minutes_of_point(24), 180 + 30);
    }

    #[test]
    fn saving_and_loading_a_log_gives_the_same_log() {
        for pushes in [0, 1, 5, 12, 13, 14, 40, 200, 1500] {
            let mut log = alive_log(pushes);
            for _ in 0..(pushes % 4) {
                log.push(SpeciesDataPoint::default());
            }
            let bytes = log.save_state_bin(2000.0);
            assert_eq!(bytes.len(), log.bytes_space(), "{pushes} pushes");
            let loaded = SpeciesLog::load_state_bin(&bytes, 2000.0, false).expect("loads");
            assert_eq!(
                loaded.total_index_of_apparition(),
                log.total_index_of_apparition()
            );
            assert_eq!(
                loaded.total_index_of_disappearance(),
                log.total_index_of_disappearance()
            );
            assert_eq!(loaded.n_points_present(), log.n_points_present());
            for (scale, index) in log.present_points() {
                let a = log.point(scale, index);
                let b = loaded.point(scale, index);
                assert_eq!(a.count, b.count, "{pushes} pushes, point {scale}/{index}");
                assert_eq!(a.energy, b.energy);
                assert!((a.avg_pos[0] - b.avg_pos[0]).abs() < 0.25);
                assert!((a.pos_std_dev[1] - b.pos_std_dev[1]).abs() < 0.25);
            }
            same_up_to_rounding(
                &loaded.save_state_bin(2000.0),
                &bytes,
                &format!("{pushes} pushes"),
            );
        }
    }

    #[test]
    fn the_exact_encoding_round_trips_every_field() {
        for pushes in [0, 3, 14, 90, 700] {
            let mut log = alive_log(pushes);
            log.push(SpeciesDataPoint::default());
            let decoded = SpeciesLog::decode_exact(&log.encode_exact()).expect("decodes");
            assert_eq!(decoded, log, "{pushes} pushes");
        }
        let template = SpeciesLog::new(true);
        assert_eq!(
            SpeciesLog::decode_exact(&template.encode_exact()).unwrap(),
            template
        );
        let bytes = alive_log(20).encode_exact();
        assert!(SpeciesLog::decode_exact(&bytes[..bytes.len() - 1]).is_err());
        assert!(SpeciesLog::decode_exact(&[]).is_err());
    }

    #[test]
    fn an_empty_log_saves_nine_bytes_as_the_game_does() {
        let bytes = SpeciesLog::new(false).save_state_bin(2000.0);
        assert_eq!(bytes, vec![0xff, 0xff, 0xff, 0xff, 0, 1, 0, 0, 0]);
        assert!(SpeciesLog::load_state_bin(&bytes, 2000.0, false).is_ok());
    }

    #[test]
    fn a_truncated_file_is_an_error_not_a_panic() {
        let bytes = alive_log(30).save_state_bin(2000.0);
        for cut in [0, 3, 5, 8, bytes.len() - 1] {
            assert!(
                SpeciesLog::load_state_bin(&bytes[..cut], 2000.0, false).is_err(),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn a_data_point_from_members_is_the_game_s_sample() {
        let bibites = [
            SpeciesMember {
                energy: 10.0,
                pos: [100.0, 0.0],
            },
            SpeciesMember {
                energy: 30.0,
                pos: [200.0, 100.0],
            },
        ];
        let eggs = [SpeciesMember {
            energy: 20.0,
            pos: [100.0, 100.0],
        }];
        let sample = SpeciesDataPoint::from_members(&bibites, &eggs);
        assert_eq!(sample.count, 3);
        assert_eq!(sample.energy, 60.0);
        assert!((sample.avg_pos[0] - 150.0).abs() < 1e-3);
        assert!((sample.avg_pos[1] - 5000.0 / 60.0).abs() < 1e-3);
        assert!(sample.pos_std_dev[0] >= 10.0 && sample.pos_std_dev[1] >= 10.0);
        // A single member: no spread information, as the constructor says.
        let single = SpeciesDataPoint::from_members(&bibites[..1], &[]);
        assert_eq!(single.pos_std_dev, [1.0, 1.0]);
        assert_eq!(single.pos_cor, 0.0);
        // Nobody: the default (not alive) sample.
        assert_eq!(
            SpeciesDataPoint::from_members(&[], &[]),
            SpeciesDataPoint::default()
        );
    }

    #[test]
    fn incorporating_a_log_adds_its_counts_and_extends_the_presence() {
        let mut survivor = alive_log(8);
        let mut pruned = SpeciesLog::new(false);
        // The pruned species lived while the survivor already did, and a little before.
        for k in 0..10 {
            pruned.push(point(1 + (k % 2) as u16));
        }
        let before: Vec<u16> = (0..8).map(|i| survivor.point(0, i).count).collect();
        survivor.incorporate(&pruned);
        for (i, count) in before.iter().enumerate() {
            assert_eq!(
                survivor.point(0, i).count,
                count + pruned.point(0, i).count,
                "point {i}"
            );
        }
        assert!(
            survivor.total_index_of_apparition() >= 9,
            "the older points are present now"
        );
        assert!(survivor.alive());
    }

    /// The species section of the `data.bin` of a real save written by the game (the 26 species of
    /// `testdata/real/saves/lol.zip`), cut out with:
    /// `d = zipfile.ZipFile('lol.zip').read('data.bin')`, the third length-prefixed block of it. The folder
    /// is not part of the repository (real game data), so the test is skipped where it is missing.
    fn lol_species() -> Option<Vec<u8>> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/real/lol_species_section.bin"
        );
        let bytes = std::fs::read(path).ok();
        if bytes.is_none() {
            eprintln!("skipped: {path} is not there");
        }
        bytes
    }

    /// Every entry of the real section loads and saves back to the same file, bit for bit but for the
    /// positions' rounding (the game quantises them against the simulation size).
    #[test]
    fn the_games_own_species_logs_load_and_save_back() {
        let Some(section) = lol_species() else {
            return;
        };
        let mut at = 0;
        let mut entries = 0;
        while at < section.len() {
            let id = i64::from_le_bytes(section[at..at + 8].try_into().unwrap());
            let length = i32::from_le_bytes(section[at + 8..at + 12].try_into().unwrap()) as usize;
            let payload = &section[at + 12..at + 12 + length];
            let log = SpeciesLog::load_state_bin(payload, 2000.0, false)
                .unwrap_or_else(|error| panic!("species {id}: {error}"));
            assert_eq!(
                log.bytes_space(),
                length,
                "species {id}: the length the game wrote"
            );
            let saved = log.save_state_bin(2000.0);
            assert_eq!(saved.len(), payload.len(), "species {id}");
            // The structure, the counts and the energies come back exactly; positions to a unit.
            same_up_to_rounding(&saved, payload, &format!("species {id}"));
            // Presence marks agree with the counts of present points.
            let present: u32 = log.n_points_present();
            if present > 0 {
                assert_eq!(
                    log.total_index_of_apparition() - log.total_index_of_disappearance(),
                    log.present_points().len() as i32,
                    "species {id}: the present points fill the span between the marks"
                );
            }
            at += 12 + length;
            entries += 1;
        }
        assert_eq!(entries, 26);
        assert_eq!(at, section.len());
    }
}
