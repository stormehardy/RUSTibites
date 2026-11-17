//! The genes stream of the Gene Analysis page: how the genes of the population were distributed, once per simulated hour
//! (`Utility/GenesBucketsStreamGroup.cs`, `GenesBuckets.cs`, `GenesBucketPoint.cs`, `LogLikeArraySerial.cs`).
//!
//! A [`GenesBucketPoint`] is the histogram of one gene (20 buckets of equal width over a range the population fits in);
//! [`GenesBuckets`] holds one for each of the 35 genes. [`GenesLog`] keeps the points of the last hours at full resolution and, as
//! they get older, merges them into coarser ones (`DataLogger.SerialHeavyConfig`).

use std::ops::Index;

use crate::datalog::LogLikeFormat;
use crate::genome::{GENE_DEFS, N_GENES};

/// Buckets of one gene's histogram (`GenesBucketPoint.N`).
pub const N_BUCKETS: usize = 20;

/// Bytes of one gene's point in `data.bin`: the range (`f32` twice) and the buckets (`u16` each) (`GenesBucketPoint.ByteSize`).
const POINT_BYTES: usize = 8 + 2 * N_BUCKETS;

/// `DataLogger.SerialHeavyConfig` (acquisition: hours): the number of points kept at each scale and how many of a scale's points
/// that fall out of it make one point of the next, coarser one.
pub const SERIAL_HEAVY: [LogLikeFormat; 7] = [
    LogLikeFormat::new(6, 3),
    LogLikeFormat::new(6, 4),
    LogLikeFormat::new(8, 2),
    LogLikeFormat::new(9, 7),
    LogLikeFormat::new(6, 4),
    LogLikeFormat::new(8, 2),
    LogLikeFormat::new(8, 0),
];

/// The histogram of one gene (`GenesBucketPoint`): the buckets cover `min` to `min + span / 5 * ceil(...)` of the gene's range, the
/// first one starting at `min`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GenesBucketPoint {
    /// Where the first bucket starts.
    pub min: f32,
    /// The largest value the buckets were made for (the bucket range itself follows from `min` and `max`, see
    /// [`Self::bucket_width`]).
    pub max: f32,
    /// How many bibites fall in each bucket.
    pub data: [u16; N_BUCKETS],
}

impl GenesBucketPoint {
    /// `new GenesBucketPoint(int n)`: nothing counted, the range 0 to 1.
    pub const EMPTY: Self = Self {
        min: 0.0,
        max: 1.0,
        data: [0; N_BUCKETS],
    };

    /// `new GenesBucketPoint(BibiteGenes[] genes, int gene)`: the histogram of gene `gene` over `genes`. The range is the span of the
    /// values widened to hundredths of the gene's range, at least a fifth of it, and moved inside the gene's range.
    #[must_use]
    pub fn from_genes(genes: &[[f32; N_GENES]], gene: usize) -> Self {
        let values: Vec<f32> = genes.iter().map(|genes| genes[gene]).collect();
        Self::from_values(&values, gene)
    }

    /// The same from the values of gene `gene` alone, one per bibite.
    #[must_use]
    pub fn from_values(values: &[f32], gene: usize) -> Self {
        let def = &GENE_DEFS[gene];
        let span = def.span();
        let min_value = def.min;
        if values.is_empty() {
            return Self {
                min: def.min,
                max: def.max,
                data: [0; N_BUCKETS],
            };
        }
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for &value in values {
            min = min.min(value);
            max = max.max(value);
        }
        min = min_value + span * (100.0_f32 * (min - min_value) / span).floor() / 100.0;
        max = min_value + span * (100.0_f32 * (max - min_value) / span).ceil() / 100.0;
        let width = ceil_to_int(5.0_f32 * (max - min) / span).max(1) as f32 / 5.0 * span;
        min = min.min(def.max - width);
        max = max.max(def.min + width);
        let mut data = [0_u16; N_BUCKETS];
        for &value in values {
            let index = floor_to_int(20.0_f32 * (value - min) / width);
            // A value just under `min` (rounding) goes in the first bucket; the game would index out of range there.
            data[index.clamp(0, N_BUCKETS as i32 - 1) as usize] += 1;
        }
        Self { min, max, data }
    }

    /// The width of one bucket of this point when it is drawn (`BarGraph.UpdateGraphData`): a hundredth of the gene's range for
    /// every fifth of it the values span (`num4`).
    #[must_use]
    pub fn bucket_width(&self, gene: usize) -> f32 {
        let span = GENE_DEFS[gene].span();
        ceil_to_int(5.0_f32 * (self.max - self.min) / span).max(1) as f32 / 100.0 * span
    }
}

/// `Mathf.CeilToInt`.
fn ceil_to_int(value: f32) -> i32 {
    value.ceil() as i32
}

/// `Mathf.FloorToInt`.
fn floor_to_int(value: f32) -> i32 {
    value.floor() as i32
}

/// The histograms of every gene at one moment (`GenesBuckets`).
#[derive(Clone, Debug, PartialEq)]
pub struct GenesBuckets {
    /// One point per gene, in the genes' order.
    pub genes: Vec<GenesBucketPoint>,
}

impl GenesBuckets {
    /// `new GenesBuckets(int n)`: every gene's point empty.
    #[must_use]
    pub fn blank() -> Self {
        Self {
            genes: vec![GenesBucketPoint::EMPTY; N_GENES],
        }
    }

    /// `new GenesBuckets(BibiteGenes[] genes)`: the histograms of the genes of a population.
    #[must_use]
    pub fn from_genes(genes: &[[f32; N_GENES]]) -> Self {
        Self {
            genes: (0..N_GENES)
                .map(|gene| GenesBucketPoint::from_genes(genes, gene))
                .collect(),
        }
    }
}

impl Index<usize> for GenesBuckets {
    type Output = GenesBucketPoint;

    fn index(&self, gene: usize) -> &GenesBucketPoint {
        &self.genes[gene]
    }
}

/// `GenesBucketsStreamGroup.CompressGeneBuckets`: the `n` points at the given slots of `slots` made one. For each gene the range
/// becomes the union of theirs and every point's buckets are shared out over the new ones (a bucket that straddles two new ones
/// gives each its share), the sums divided by `n`. The game lays the source buckets out with the NEW range's bucket width, so the
/// counts of points with other ranges are not conserved; this is ported as it is.
fn compress_gene_buckets(
    data: &[GenesBuckets],
    slot_of: impl Fn(usize) -> usize,
    n: usize,
) -> GenesBuckets {
    let buckets = N_BUCKETS as f32;
    let slots: Vec<usize> = (0..n).map(&slot_of).collect();
    let mut result = GenesBuckets::blank();
    for gene in 0..N_GENES {
        let span = GENE_DEFS[gene].span();
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for &slot in &slots {
            min = min.min(data[slot][gene].min);
            max = max.max(data[slot][gene].max);
        }
        let mut point = GenesBucketPoint::EMPTY;
        point.min = min;
        point.max = max;
        let width = ceil_to_int(5.0_f32 * (max - min) / span).max(1) as f32 / 5.0 * span / buckets;
        let mut sums = [0.0_f32; N_BUCKETS];
        for &slot in &slots {
            let source = &data[slot][gene];
            let source_width =
                (5.0_f32 * (max - min) / span).ceil().max(1.0) / 5.0 * span / buckets;
            let offset = (source.min - min) / width;
            let ratio = source_width / width;
            for m in 0..N_BUCKETS {
                let at = ceil_to_int(offset + m as f32 * ratio);
                let share =
                    (min + at as f32 * width - source.min - m as f32 * source_width) / source_width;
                if at > N_BUCKETS as i32 {
                    break;
                }
                let count = f32::from(source.data[m]);
                sums[(at - 1).max(0) as usize] += share * count;
                if at < N_BUCKETS as i32 {
                    sums[at as usize] += (1.0 - share) * count;
                }
            }
        }
        for (bucket, sum) in sums.iter().enumerate() {
            point.data[bucket] = (sum / n as f32) as u16;
        }
        result.genes[gene] = point;
    }
    result
}

/// `LogLikeArraySerial<GenesBuckets>`: a log whose scales hold the points of the finest one's past, each scale `size` points and the
/// room (`feed_ratio` more slots) to collect the points that fall out of it until there are enough to make one point of the next.
#[derive(Clone, Debug)]
pub struct GenesLog {
    formats: Vec<LogLikeFormat>,
    slots: Vec<GenesBuckets>,
    scale_offsets: Vec<usize>,
    index_offsets: Vec<usize>,
    push_counters: Vec<u8>,
    n_data_at_scale: Vec<u8>,
    n_points: usize,
    n_data: usize,
}

impl Default for GenesLog {
    fn default() -> Self {
        Self::new()
    }
}

impl GenesLog {
    /// An empty log of the game's configuration ([`SERIAL_HEAVY`]).
    #[must_use]
    pub fn new() -> Self {
        Self::with_formats(&SERIAL_HEAVY)
    }

    /// An empty log of another configuration (its last scale must be bounded).
    #[must_use]
    pub fn with_formats(formats: &[LogLikeFormat]) -> Self {
        let n = formats.len();
        let mut scale_offsets = vec![0; n];
        let mut total = 0;
        for (i, format) in formats.iter().enumerate() {
            scale_offsets[i] = total;
            total += format.size + format.feed_ratio;
        }
        Self {
            formats: formats.to_vec(),
            slots: vec![GenesBuckets::blank(); total],
            scale_offsets,
            index_offsets: vec![0; n],
            push_counters: vec![0; n],
            n_data_at_scale: vec![0; n],
            n_points: 0,
            n_data: 0,
        }
    }

    /// Number of scales.
    #[must_use]
    pub fn n_scales(&self) -> usize {
        self.formats.len()
    }

    /// The points of the log (`nData`): what the Gene Analysis page's time slider can show.
    #[must_use]
    pub fn n_data(&self) -> usize {
        self.n_data
    }

    /// The points each scale holds (`nDataAtScale`).
    #[must_use]
    pub fn n_data_at_scale(&self, scale: usize) -> usize {
        usize::from(self.n_data_at_scale[scale])
    }

    /// The slots a scale's ring has (its points and the room for those on their way to the next scale).
    fn scale_size(&self, scale: usize) -> usize {
        self.formats[scale].size + self.formats[scale].feed_ratio
    }

    /// The slot of the `i`th newest entry of a scale's ring (`ToCircularIndexOfScale`).
    fn slot_of(&self, scale: usize, i: usize) -> usize {
        self.scale_offsets[scale] + (self.index_offsets[scale] + i) % self.scale_size(scale)
    }

    /// `Push`: a new point goes in at the finest scale. While a scale has room it stays there; once it is full, the oldest point falls
    /// into the room beside it and, when `feed_ratio` have fallen, they are made one point and pushed into the next scale.
    pub fn push(&mut self, point: GenesBuckets) {
        let n_scales = self.n_scales();
        let mut value = point;
        let mut cumulative = 0;
        for scale in 0..n_scales {
            let scale_size = self.scale_size(scale);
            let data_size = self.formats[scale].size;
            let head = match self.index_offsets[scale] {
                0 => scale_size - 1,
                head => head - 1,
            };
            let offset = self.scale_offsets[scale];
            self.index_offsets[scale] = head;
            self.slots[offset + head] = value;
            cumulative += scale_size;
            if self.n_points < cumulative {
                self.n_points += 1;
            }
            if usize::from(self.n_data_at_scale[scale]) < data_size {
                self.n_data_at_scale[scale] += 1;
                self.n_data += 1;
                return;
            }
            if scale + 1 >= n_scales {
                return;
            }
            let feed = self.formats[scale].feed_ratio;
            self.push_counters[scale] += 1;
            if usize::from(self.push_counters[scale]) < feed {
                return;
            }
            self.push_counters[scale] = 0;
            let accumulators = head + data_size;
            value = compress_gene_buckets(
                &self.slots,
                |o| offset + (accumulators + o) % scale_size,
                feed,
            );
        }
    }

    /// `this[int i]`: the `i`th newest point of the log across the scales (0 = the newest), `None` past the points it holds.
    #[must_use]
    pub fn get(&self, i: usize) -> Option<&GenesBuckets> {
        let mut i = i;
        for scale in 0..self.n_scales() {
            let size = self.formats[scale].size;
            if i < size {
                return (i < usize::from(self.n_data_at_scale[scale]))
                    .then(|| &self.slots[self.slot_of(scale, i)]);
            }
            i -= size;
        }
        None
    }

    /// `GetAllData(includeAccumulators)`: the entries of every scale, the newest first; with the accumulators every slot that was
    /// ever written, otherwise only the points.
    fn all_entries(&self, include_accumulators: bool) -> Vec<&GenesBuckets> {
        let total = if include_accumulators {
            self.n_points
        } else {
            self.n_data
        };
        let mut entries = Vec::with_capacity(total);
        for scale in 0..self.n_scales() {
            let size = if include_accumulators {
                self.scale_size(scale)
            } else {
                self.formats[scale].size
            };
            for j in 0..size.min(total - entries.len()) {
                entries.push(&self.slots[self.slot_of(scale, j)]);
            }
        }
        entries
    }

    /// `GenesBucketsStreamGroup.SaveStateBin`: the slot count, the point count, the buckets per gene and the genes (`i32` each), every
    /// slot's genes (`min` and `max` as `f32`, the buckets as `u16`), the push counters of all scales but the last and the point
    /// count of every scale (a byte each).
    #[must_use]
    pub fn save_state_bin(&self) -> Vec<u8> {
        let entries = self.all_entries(true);
        let n_scales = self.n_scales();
        let mut out =
            Vec::with_capacity(16 + entries.len() * N_GENES * POINT_BYTES + 2 * n_scales - 1);
        out.extend_from_slice(&(entries.len() as i32).to_le_bytes());
        out.extend_from_slice(&(self.n_data as i32).to_le_bytes());
        out.extend_from_slice(&(N_BUCKETS as i32).to_le_bytes());
        out.extend_from_slice(&(N_GENES as i32).to_le_bytes());
        for entry in entries {
            for point in &entry.genes {
                out.extend_from_slice(&point.min.to_le_bytes());
                out.extend_from_slice(&point.max.to_le_bytes());
                for bucket in point.data {
                    out.extend_from_slice(&bucket.to_le_bytes());
                }
            }
        }
        out.extend_from_slice(&self.push_counters[..n_scales - 1]);
        out.extend_from_slice(&self.n_data_at_scale);
        out
    }

    /// `GenesBucketsStreamGroup.LoadStateBin` (the format of game versions from 0.6.0a9 on): the slots are taken as they come, the
    /// first one the newest of the finest scale, and every ring starts at its first slot. `false` (and nothing changed) when the
    /// bytes do not fit this log.
    pub fn load_state_bin(&mut self, bytes: &[u8]) -> bool {
        let n_scales = self.n_scales();
        if bytes.len() < 16 {
            return false;
        }
        let int = |at: usize| {
            i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let (Ok(n_points), Ok(n_data), Ok(buckets), Ok(genes)) = (
            usize::try_from(int(0)),
            usize::try_from(int(4)),
            usize::try_from(int(8)),
            usize::try_from(int(12)),
        ) else {
            return false;
        };
        if buckets != N_BUCKETS || genes > N_GENES || n_points > self.slots.len() {
            return false;
        }
        let points_end = 16 + n_points * genes * POINT_BYTES;
        if bytes.len() < points_end + 2 * n_scales - 1 {
            return false;
        }
        for slot in &mut self.slots {
            *slot = GenesBuckets::blank();
        }
        let float = |at: usize| {
            f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let mut at = 16;
        for slot in &mut self.slots[..n_points] {
            for point in &mut slot.genes[..genes] {
                point.min = float(at);
                point.max = float(at + 4);
                for (bucket, value) in point.data.iter_mut().enumerate() {
                    let b = at + 8 + 2 * bucket;
                    *value = u16::from_le_bytes([bytes[b], bytes[b + 1]]);
                }
                at += POINT_BYTES;
            }
        }
        for (counter, byte) in self
            .push_counters
            .iter_mut()
            .zip(&bytes[points_end..points_end + n_scales - 1])
        {
            *counter = *byte;
        }
        self.push_counters[n_scales - 1] = 0;
        self.n_data_at_scale
            .copy_from_slice(&bytes[points_end + n_scales - 1..points_end + 2 * n_scales - 1]);
        self.index_offsets.iter_mut().for_each(|offset| *offset = 0);
        self.n_points = n_points;
        self.n_data = n_data;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A population of `values.len()` bibites that differ in gene `gene` only.
    fn population(gene: usize, values: &[f32]) -> Vec<[f32; N_GENES]> {
        values
            .iter()
            .map(|&value| {
                let mut genes = [0.0_f32; N_GENES];
                for (slot, def) in genes.iter_mut().zip(GENE_DEFS.iter()) {
                    *slot = def.default;
                }
                genes[gene] = value;
                genes
            })
            .collect()
    }

    #[test]
    fn the_range_widens_to_hundredths_of_the_genes_range_and_at_least_a_fifth() {
        // Gene 0, the lay time: 1 to 51, a span of 50. The values 4 and 11 widen to 4 and 11 (multiples of 0.5), and a fifth of the
        // span is 10: the buckets are 0.5 wide and run from 4 to 14, the values 4, 4.2 and 11 in the buckets 0, 0 and 14.
        let point = GenesBucketPoint::from_genes(&population(0, &[4.0, 4.2, 11.0]), 0);
        assert_eq!((point.min, point.max), (4.0, 11.0));
        assert_eq!(point.bucket_width(0), 0.5);
        let mut expected = [0_u16; N_BUCKETS];
        expected[0] = 2;
        expected[14] = 1;
        assert_eq!(point.data, expected);
        // Values 11 apart need two fifths of the span: the buckets are 1 wide and `max` is raised to the end of the range (1 + 20),
        // as the real game's points show.
        let point = GenesBucketPoint::from_genes(&population(0, &[1.0, 12.0]), 0);
        assert_eq!((point.min, point.max), (1.0, 21.0));
        assert_eq!(point.bucket_width(0), 1.0);
        assert_eq!(point.data[0], 1);
        assert_eq!(point.data[11], 1);
        assert_eq!(point.data.iter().map(|n| u32::from(*n)).sum::<u32>(), 2);
    }

    #[test]
    fn the_range_stays_inside_the_genes_range() {
        // The largest lay times: the buckets (a fifth of the range, 10) are moved down so that they end at the gene's maximum; 50 is
        // in the bucket 18 of 41 to 51, 51 in the last.
        let point = GenesBucketPoint::from_genes(&population(0, &[50.0, 51.0]), 0);
        assert_eq!((point.min, point.max), (41.0, 51.0));
        assert_eq!(point.data.iter().map(|n| u32::from(*n)).sum::<u32>(), 2);
        assert_eq!((point.data[18], point.data[19]), (1, 1));
        // No population: the gene's whole range, nothing counted.
        let point = GenesBucketPoint::from_genes(&[], 0);
        assert_eq!(
            (point.min, point.max, point.data),
            (1.0, 51.0, [0; N_BUCKETS])
        );
    }

    #[test]
    fn every_bibite_is_counted_once_in_every_gene() {
        let genes: Vec<[f32; N_GENES]> = (0..37)
            .map(|k| {
                let mut genes = [0.0_f32; N_GENES];
                for (slot, def) in genes.iter_mut().zip(GENE_DEFS.iter()) {
                    *slot = def.min + def.span() * ((k * 7 + 3) % 37) as f32 / 36.0;
                }
                genes
            })
            .collect();
        let buckets = GenesBuckets::from_genes(&genes);
        assert_eq!(buckets.genes.len(), N_GENES);
        for point in &buckets.genes {
            assert_eq!(point.data.iter().map(|n| u32::from(*n)).sum::<u32>(), 37);
        }
    }

    /// A population that has the same value in every gene it is built for.
    fn flat(value: f32, n: usize) -> GenesBuckets {
        let genes = vec![
            {
                let mut genes = [0.0_f32; N_GENES];
                for (slot, def) in genes.iter_mut().zip(GENE_DEFS.iter()) {
                    *slot = def.min + def.span() * value;
                }
                genes
            };
            n
        ];
        GenesBuckets::from_genes(&genes)
    }

    #[test]
    fn a_log_keeps_the_newest_first_and_hands_old_points_to_the_next_scale() {
        // Two scales: 3 points then 3 more (2 of them make one).
        let mut log = GenesLog::with_formats(&[LogLikeFormat::new(3, 2), LogLikeFormat::new(3, 0)]);
        let points: Vec<GenesBuckets> = (1..=12).map(|k| flat(k as f32 / 20.0, k)).collect();
        let sum =
            |buckets: &GenesBuckets| -> u32 { buckets[0].data.iter().map(|n| u32::from(*n)).sum() };
        for (k, point) in points.iter().take(3).enumerate() {
            log.push(point.clone());
            assert_eq!(log.n_data(), k + 1);
        }
        assert_eq!(sum(log.get(0).unwrap()), 3, "the newest first");
        assert_eq!(sum(log.get(2).unwrap()), 1);
        assert!(log.get(3).is_none());
        // The fourth and fifth fall into the room beside the finest scale; together they make the first point of the next.
        log.push(points[3].clone());
        assert_eq!((log.n_data(), log.n_data_at_scale(1)), (3, 0));
        log.push(points[4].clone());
        assert_eq!((log.n_data(), log.n_data_at_scale(1)), (4, 1));
        assert_eq!(sum(log.get(0).unwrap()), 5);
        assert_eq!(sum(log.get(2).unwrap()), 3);
        assert!(log.get(3).is_some(), "the first point of the coarser scale");
        // Enough pushes fill everything and nothing grows any more.
        for point in points.iter().skip(5) {
            log.push(point.clone());
        }
        assert_eq!(log.n_data(), 6);
        assert!(log.get(6).is_none());
        assert_eq!(sum(log.get(0).unwrap()), 12);
    }

    #[test]
    fn merging_identical_points_gives_the_same_point() {
        let mut log = GenesLog::with_formats(&[LogLikeFormat::new(2, 3), LogLikeFormat::new(2, 0)]);
        let point = flat(0.3, 9);
        for _ in 0..5 {
            log.push(point.clone());
        }
        // Two points stay, three fell out and became one of the next scale.
        assert_eq!((log.n_data_at_scale(0), log.n_data_at_scale(1)), (2, 1));
        let merged = log.get(2).unwrap();
        for gene in 0..N_GENES {
            assert_eq!(merged[gene].data, point[gene].data, "gene {gene}");
            assert_eq!(
                (merged[gene].min, merged[gene].max),
                (point[gene].min, point[gene].max)
            );
        }
    }

    #[test]
    fn merging_points_with_other_ranges_takes_the_union_and_shares_the_counts() {
        let mut log = GenesLog::with_formats(&[LogLikeFormat::new(1, 2), LogLikeFormat::new(2, 0)]);
        let low = GenesBuckets::from_genes(&population(0, &[2.0, 2.1]));
        let high = GenesBuckets::from_genes(&population(0, &[12.5, 12.5, 12.6, 12.7]));
        assert_eq!((low[0].min, low[0].max), (2.0, 11.0));
        assert_eq!((high[0].min, high[0].max), (12.5, 13.0));
        // The first point is pushed out of the finest scale by the next, the second by the third: those two are merged, the third stays.
        log.push(high.clone());
        log.push(low.clone());
        log.push(GenesBuckets::blank());
        let merged = log.get(1).expect("the merged point");
        // The union of the two ranges, 2 to 13: 2 fifths of the span, so the buckets are 1 wide.
        assert_eq!((merged[0].min, merged[0].max), (2.0, 13.0));
        // The low point's two bibites are in its first bucket, which stays the first. The high point's four are in a bucket that is
        // really 0.5 wide but is laid out 1 wide, and it starts half way into the merged bucket 10: it is shared between the buckets
        // 10 and 11, a half each. All divided by the 2 points merged.
        let mut expected = [0_u16; N_BUCKETS];
        expected[0] = 1;
        expected[10] = 1;
        expected[11] = 1;
        assert_eq!(merged[0].data, expected);
    }

    #[test]
    fn a_saved_log_reads_back_the_same_and_goes_on_the_same_way() {
        let formats = [
            LogLikeFormat::new(2, 2),
            LogLikeFormat::new(2, 3),
            LogLikeFormat::new(3, 0),
        ];
        let mut original = GenesLog::with_formats(&formats);
        for k in 1..=14 {
            original.push(flat((k % 9) as f32 / 10.0, k));
        }
        let bytes = original.save_state_bin();
        assert_eq!(i32::from_le_bytes(bytes[8..12].try_into().unwrap()), 20);
        assert_eq!(i32::from_le_bytes(bytes[12..16].try_into().unwrap()), 35);
        let slots = i32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), 16 + slots * N_GENES * 48 + 2 * 3 - 1);
        let mut loaded = GenesLog::with_formats(&formats);
        assert!(loaded.load_state_bin(&bytes));
        assert_eq!(loaded.n_data(), original.n_data());
        for i in 0..original.n_data() {
            assert_eq!(loaded.get(i), original.get(i), "point {i}");
        }
        assert_eq!(
            loaded.save_state_bin(),
            bytes,
            "saved again, the same bytes"
        );
        for k in 15..=30 {
            let point = flat((k % 7) as f32 / 10.0, k);
            original.push(point.clone());
            loaded.push(point);
            for i in 0..original.n_data() {
                assert_eq!(loaded.get(i), original.get(i), "push {k}, point {i}");
            }
        }
        // Bytes that do not fit change nothing.
        let mut other = GenesLog::with_formats(&formats);
        assert!(!other.load_state_bin(&bytes[..bytes.len() - 1]));
        assert!(!other.load_state_bin(&[0; 8]));
        assert_eq!(other.n_data(), 0);
        let mut bad = bytes.clone();
        bad[8] = 7;
        assert!(!other.load_state_bin(&bad), "another number of buckets");
    }

    /// The genes stream's element of a blank game's `data.bin`: no point, 20 buckets, 35 genes, the push counters of the 6 coarser
    /// scales and the point counts of the 7 scales.
    const BLANK_GENES_STREAM: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 20, 0, 0, 0, 35, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    #[test]
    fn an_empty_log_writes_the_bytes_of_a_blank_game() {
        let log = GenesLog::new();
        assert_eq!(log.save_state_bin(), BLANK_GENES_STREAM);
        let mut loaded = GenesLog::new();
        assert!(loaded.load_state_bin(&BLANK_GENES_STREAM));
        assert_eq!(loaded.n_data(), 0);
        assert!(loaded.get(0).is_none());
    }
}
