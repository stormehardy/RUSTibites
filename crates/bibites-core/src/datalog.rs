//! Multi-resolution history streams (`Utility/DataLogger.cs`).
//!
//! A [`LogLikeParallel`] keeps the newest points at full resolution and feeds
//! every `feed_ratio`-th push into the next, coarser scale, exactly like the
//! game's `LogLikeArrayParallel<T>` (`Here/Utility/LogLikeArrayParallel.cs`).
//! The [`DataLogger`] logs the streams the Graphs panel plots once per
//! simulated minute (`DataLogger.FixedUpdate`, `periodSeconds = 60`). The ages of death stream is
//! fed by a rolling Greenwald-Khanna quantile summary ([`RollingQuantile`], `Utility/GKQuantileEstimator.cs`,
//! `RollingQuantileEstimator.cs`).

/// One scale of a log-like array (`LogLikeFormat`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogLikeFormat {
    /// Number of points kept at this scale (`0` = unbounded last scale).
    pub size: usize,
    /// Number of pushes to this scale per push to the next one.
    pub feed_ratio: usize,
}

impl LogLikeFormat {
    /// `new LogLikeFormat(size, feedRatio)`.
    #[must_use]
    pub const fn new(size: usize, feed_ratio: usize) -> Self {
        Self { size, feed_ratio }
    }
}

/// `DataLogger.ParallelDetailedConfig` (acquisition: minutes).
pub const PARALLEL_DETAILED: [LogLikeFormat; 6] = [
    LogLikeFormat::new(60, 10),
    LogLikeFormat::new(60, 3),
    LogLikeFormat::new(96, 6),
    LogLikeFormat::new(112, 8),
    LogLikeFormat::new(112, 7),
    LogLikeFormat::new(104, 0),
];

/// How a block of `feed_ratio` points becomes one coarser point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compress {
    /// `GenericCompressFunc.Oldest`: the oldest of the block.
    Oldest,
    /// `DataPointCompress.Sum`: component-wise sum.
    Sum,
}

/// `LogLikeArrayParallel<T>` for points of `width` floats.
#[derive(Clone, Debug)]
pub struct LogLikeParallel {
    width: usize,
    formats: Vec<LogLikeFormat>,
    compress: Compress,
    data: Vec<f32>,
    scale_offsets: Vec<usize>,
    index_offsets: Vec<usize>,
    push_counters: Vec<usize>,
    n_at_scale: Vec<usize>,
    /// Unbounded last scale, newest first (`infiniteData`).
    infinite: Option<Vec<Vec<f32>>>,
}

impl LogLikeParallel {
    /// Builds an empty array.
    #[must_use]
    pub fn new(formats: &[LogLikeFormat], width: usize, compress: Compress) -> Self {
        let n = formats.len();
        let mut scale_offsets = vec![0; n];
        let mut total = 0;
        for (i, format) in formats.iter().enumerate() {
            scale_offsets[i] = total;
            total += format.size;
        }
        let infinite = formats
            .last()
            .is_some_and(|format| format.size == 0)
            .then(Vec::new);
        Self {
            width,
            formats: formats.to_vec(),
            compress,
            data: vec![0.0; total * width],
            scale_offsets,
            index_offsets: vec![0; n],
            push_counters: vec![0; n],
            n_at_scale: vec![0; n],
            infinite,
        }
    }

    /// Number of scales (`nScales`).
    #[must_use]
    pub fn n_scales(&self) -> usize {
        self.formats.len()
    }

    /// Points per component (`width`).
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Time per point at `scale`, in units of the acquisition period
    /// (`scaleSteps`).
    #[must_use]
    pub fn scale_step(&self, scale: usize) -> usize {
        self.formats[..scale]
            .iter()
            .map(|format| format.feed_ratio.max(1))
            .product()
    }

    /// Maximum number of points the scale can hold (`DataSizeAtScale`).
    #[must_use]
    pub fn data_size_at_scale(&self, scale: usize) -> usize {
        match &self.infinite {
            Some(points) if scale == self.n_scales() - 1 => {
                points.len().max(self.formats[scale - 1].size)
            }
            _ => self.formats[scale].size,
        }
    }

    /// Points currently stored at `scale` (`NDataAtScale`).
    #[must_use]
    pub fn n_data_at_scale(&self, scale: usize) -> usize {
        match &self.infinite {
            Some(points) if scale == self.n_scales() - 1 => points.len(),
            _ => self.n_at_scale[scale],
        }
    }

    /// `ScaleHasData`.
    #[must_use]
    pub fn scale_has_data(&self, scale: usize) -> bool {
        self.n_data_at_scale(scale) > 0
    }

    /// Point `i` (0 = newest) of `scale` (`this[scale, i]`).
    #[must_use]
    pub fn get(&self, scale: usize, i: usize) -> &[f32] {
        if let Some(points) = &self.infinite
            && scale == self.n_scales() - 1
        {
            return &points[i];
        }
        let size = self.formats[scale].size;
        let slot = self.scale_offsets[scale] + (self.index_offsets[scale] + i) % size;
        &self.data[slot * self.width..(slot + 1) * self.width]
    }

    fn compress_block(&self, scale: usize, newest_slot: usize, n: usize) -> Vec<f32> {
        let size = self.formats[scale].size;
        let offset = self.scale_offsets[scale];
        // `accOffset = newest + dataSize`; `remap(o) = offset + (acc + o) % size`
        // with `scaleSize == dataSize`, i.e. `o` counts back from the newest.
        let slot_of = |o: usize| offset + (newest_slot + o) % size;
        match self.compress {
            Compress::Oldest => {
                let slot = slot_of(n - 1);
                self.data[slot * self.width..(slot + 1) * self.width].to_vec()
            }
            Compress::Sum => {
                let mut out = vec![0.0_f32; self.width];
                for o in 0..n {
                    let slot = slot_of(o);
                    for (j, value) in out.iter_mut().enumerate() {
                        *value += self.data[slot * self.width + j];
                    }
                }
                out
            }
        }
    }

    /// `GetAllData`: every scale's points, one scale after the other, the newest of a scale first.
    #[must_use]
    pub fn all_points(&self) -> Vec<Vec<f32>> {
        let mut points = Vec::new();
        for scale in 0..self.n_scales() {
            for i in 0..self.n_at_scale[scale] {
                points.push(self.get(scale, i).to_vec());
            }
        }
        points
    }

    /// `DataParallelStreamGroup.SaveStateBin`: the point count (`i32`), the width and the scale count (a byte each), the floats stream
    /// after stream (`array[stream * nData + point]`), the push counters of all scales but the last and the point count of every scale
    /// (a byte each).
    #[must_use]
    pub fn save_state_bin(&self) -> Vec<u8> {
        let points = self.all_points();
        let n_scales = self.n_scales();
        let mut out = Vec::with_capacity(6 + points.len() * self.width * 4 + 2 * n_scales);
        out.extend_from_slice(&(points.len() as i32).to_le_bytes());
        out.push(self.width as u8);
        out.push(n_scales as u8);
        for stream in 0..self.width {
            for point in &points {
                out.extend_from_slice(&point[stream].to_le_bytes());
            }
        }
        for counter in &self.push_counters[..n_scales - 1] {
            out.push(*counter as u8);
        }
        for n in &self.n_at_scale {
            out.push(*n as u8);
        }
        out
    }

    /// `DataParallelStreamGroup.LoadStateBin` (the format of game versions from 0.6.0a9 on). The points of a scale are put at the
    /// start of its ring, newest first, and the loading stops at the first scale without points; `false` (and nothing changed)
    /// when the bytes do not fit this array (another width or scale count, or too short).
    pub fn load_state_bin(&mut self, bytes: &[u8]) -> bool {
        let n_scales = self.n_scales();
        if bytes.len() < 6 {
            return false;
        }
        let n = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let (width, scales) = (usize::from(bytes[4]), usize::from(bytes[5]));
        let Ok(n) = usize::try_from(n) else {
            return false;
        };
        if width != self.width || scales != n_scales {
            return false;
        }
        let floats = 6 + width * n * 4;
        if bytes.len() < floats + (n_scales - 1) + n_scales {
            return false;
        }
        let float_at = |index: usize| {
            let at = 6 + index * 4;
            f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let push_counters: Vec<usize> = (0..n_scales - 1)
            .map(|i| usize::from(bytes[floats + i]))
            .collect();
        let n_at_scale: Vec<usize> = (0..n_scales)
            .map(|i| usize::from(bytes[floats + n_scales - 1 + i]))
            .collect();
        self.data.iter_mut().for_each(|value| *value = 0.0);
        self.index_offsets.iter_mut().for_each(|offset| *offset = 0);
        let mut taken = 0;
        for (scale, &count) in n_at_scale.iter().enumerate() {
            if count == 0 {
                break;
            }
            for k in 0..count.min(self.formats[scale].size) {
                if taken + k >= n {
                    break;
                }
                let slot = self.scale_offsets[scale] + k;
                for stream in 0..width {
                    self.data[slot * width + stream] = float_at(stream * n + taken + k);
                }
            }
            taken += count;
        }
        for (counter, loaded) in self.push_counters.iter_mut().zip(&push_counters) {
            *counter = *loaded;
        }
        self.n_at_scale = n_at_scale;
        true
    }

    /// `Push`: logs a point at the finest scale and cascades into coarser ones.
    pub fn push(&mut self, point: &[f32]) {
        debug_assert_eq!(point.len(), self.width);
        let n_scales = self.n_scales();
        let mut value = point.to_vec();
        // `beforeDataPushedToScale(val, 0)` only matters when the very first
        // scale is the unbounded one.
        if n_scales == 1 {
            if let Some(points) = &mut self.infinite {
                points.insert(0, value);
            }
            return;
        }
        for i in 0..n_scales {
            if self.infinite.is_some() && i == n_scales - 1 {
                break;
            }
            let size = self.formats[i].size;
            let mut slot = self.index_offsets[i];
            slot = if slot == 0 { size - 1 } else { slot - 1 };
            self.index_offsets[i] = slot;
            let at = (self.scale_offsets[i] + slot) * self.width;
            self.data[at..at + self.width].copy_from_slice(&value);
            if self.n_at_scale[i] < size {
                self.n_at_scale[i] += 1;
            }
            if i >= n_scales - 1 {
                return;
            }
            let feed = self.formats[i].feed_ratio;
            self.push_counters[i] += 1;
            if self.push_counters[i] < feed {
                return;
            }
            self.push_counters[i] = 0;
            value = self.compress_block(i, slot, feed);
            if i + 1 == n_scales - 1
                && let Some(points) = &mut self.infinite
            {
                points.insert(0, value.clone());
            }
        }
    }
}

/// Label/colour description of one component (`DataStreamDescription`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamDescription {
    /// Legend label.
    pub label: &'static str,
    /// The curve's tooltip in the Curves dropdown (`description`).
    pub description: &'static str,
    /// Line colour (sRGB 0..=1).
    pub color: [f32; 3],
}

/// A named group of parallel streams plus its history.
#[derive(Clone, Debug)]
pub struct StreamGroup {
    /// Panel title (`title`).
    pub title: &'static str,
    /// The Data dropdown's tooltip of the group (`description`).
    pub description: &'static str,
    /// Per-component descriptions.
    pub streams: Vec<StreamDescription>,
    /// History.
    pub history: LogLikeParallel,
}

impl StreamGroup {
    fn new(
        title: &'static str,
        description: &'static str,
        streams: Vec<StreamDescription>,
        compress: Compress,
    ) -> Self {
        let width = streams.len();
        Self {
            title,
            description,
            streams,
            history: LogLikeParallel::new(&PARALLEL_DETAILED, width, compress),
        }
    }
}

const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
const GREEN: [f32; 3] = [0.0, 1.0, 0.0];
const RED: [f32; 3] = [1.0, 0.0, 0.0];
/// `Color.yellow` (1, 0.92, 0.016): the game's yellow is not (1, 1, 0).
const YELLOW: [f32; 3] = [1.0, 0.921_568_6, 0.015_686_275];
const BLUE: [f32; 3] = [0.0, 0.0, 1.0];
const LIGHT_GREEN: [f32; 3] = [0.5, 1.0, 0.5];
const DARK_GREEN: [f32; 3] = [0.0, 0.5, 0.0];

/// Simulated seconds between logged points (`DataLogger.periodSeconds`).
pub const LOG_PERIOD_SECONDS: f32 = 60.0;

/// Index of each group in [`DataLogger::groups`]: the order of `DataLogger.parallelDataStreams`,
/// which is the order of the Graphs panel's Data dropdown.
pub mod group {
    /// `biomassDataStreamGroup`: bibites+eggs, plants, meats, free.
    pub const BIOMASS: usize = 0;
    /// `countDataStreamGroup`: bibites+eggs, plants, meats.
    pub const COUNT: usize = 1;
    /// `ageDataStreamGroup`: 25th percentile, median, 75th percentile, max.
    pub const AGE: usize = 2;
    /// `brainSizeDataStreamGroup`: min/mean/max nodes, min/mean/max connections.
    pub const BRAIN_SIZE: usize = 3;
    /// `deathAgeDataStreamGroup`: 25th percentile, median, 75th percentile, max age of death.
    pub const DEATH_AGE: usize = 4;
    /// `eggsLaidDataStreamGroup`: median, p70, p80, p90, max, mean.
    pub const EGGS_LAID: usize = 5;
    /// `birthDeathDataStreamGroup`: births, deaths since the last point.
    pub const BIRTH_DEATH: usize = 6;
}

/// The quantiles the ages of death are reported at (`BibiteTracker.GetAgeOfDeathData`).
const DEATH_AGE_QUANTILES: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

/// `DataLogger`: the parallel streams plotted by the Graphs panel (the genes bucket stream is not
/// implemented).
#[derive(Clone, Debug)]
pub struct DataLogger {
    /// Stream groups, indexed by the [`group`] constants.
    pub groups: Vec<StreamGroup>,
    /// `BibiteTracker.deathAgesQuantiles`: the ages (hours) the dead bibites died at, over a
    /// rolling hour.
    pub death_ages: RollingQuantile,
    /// Cumulative births at the previous log (for the birth/death deltas).
    pub last_births_total: u64,
    /// Tracked deaths since the previous log (`BibiteTracker.nDeath`).
    pub deaths_since_log: u32,
    /// Hatches since the previous log (`BibiteTracker.BibiteBirth` is only
    /// called by `StartBody`, i.e. for hatched bibites).
    pub hatches_since_log: u32,
    /// `DataLogger.nMinutes`: the minutes since the last full hour (the genes stream is logged when an hour is full).
    pub n_minutes: i32,
    /// The genes stream (`DataLogger.genesStreamGroup`): the distribution of every gene once per simulated hour.
    pub genes: crate::genes_log::GenesLog,
    progress: f32,
}

impl Default for DataLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl DataLogger {
    /// Creates an empty logger.
    #[must_use]
    pub fn new() -> Self {
        let d = |label, description, color| StreamDescription {
            label,
            description,
            color,
        };
        let age_curves = |verb: &'static str, last: &'static str| {
            let younger = if verb == "die" {
                "The age at which 25% of the bibites in your simulation die younger than this."
            } else {
                "The age at which 25% of the bibites in your simulation are younger than this."
            };
            let median = if verb == "die" {
                "The age at which 50% of the bibites in your simulation die younger than this."
            } else {
                "The age at which 50% of the bibites in your simulation are younger than this."
            };
            let older = if verb == "die" {
                "The age at which 25% of the bibites in your simulation die older than this."
            } else {
                "The age at which 25% of the bibites in your simulation are older than this."
            };
            vec![
                d("Bottom 25th percentile Age", younger, WHITE),
                d("Median Age", median, LIGHT_GREEN),
                d("Top 25th percentile Age", older, GREEN),
                d("Max Age", last, DARK_GREEN),
            ]
        };
        let groups = vec![
            StreamGroup::new(
                "Biomass Data",
                "The historic biomass levels (total energy equivalence) of your simulation",
                vec![
                    d(
                        "Bibites",
                        "The biomass level of bibites (and eggs) in the simulation",
                        WHITE,
                    ),
                    d(
                        "Plants",
                        "The biomass level of plant pellets in your simulation",
                        GREEN,
                    ),
                    d(
                        "Meats",
                        "The biomass level of meat pellets in your simulation",
                        RED,
                    ),
                    d(
                        "Free",
                        "The free biomass level of your simulation. This represents the available biomass to grow more plants if need be.",
                        YELLOW,
                    ),
                ],
                Compress::Oldest,
            ),
            StreamGroup::new(
                "Entities Count Data",
                "The historic count of each type of entities in your simulation",
                vec![
                    d(
                        "Bibites",
                        "The number of bibites (and eggs) in the simulation",
                        WHITE,
                    ),
                    d(
                        "Plants",
                        "The number of plant pellets in the simulation",
                        GREEN,
                    ),
                    d("Meats", "The number of meat pellets in the simulation", RED),
                ],
                Compress::Oldest,
            ),
            StreamGroup::new(
                "Age Data",
                "The age distribution of bibites in your simulation.",
                age_curves("are", "The age of the oldest bibite in the sim"),
                Compress::Oldest,
            ),
            StreamGroup::new(
                "Brain Size Data",
                "The brain size, for both nodes and connections, range of you bibites over time",
                vec![
                    d(
                        "Min Nodes",
                        "The smallest number of brain nodes in the population",
                        LIGHT_GREEN,
                    ),
                    d(
                        "Mean Nodes",
                        "The average number of brain nodes in the population",
                        GREEN,
                    ),
                    d(
                        "Max Nodes",
                        "The biggest number of brain nodes in the population",
                        DARK_GREEN,
                    ),
                    d(
                        "Min Connections",
                        "The smallest number of brain connections in the population",
                        [0.65, 0.65, 1.0],
                    ),
                    d(
                        "Mean Connections",
                        "The average number of brain connections in the population",
                        BLUE,
                    ),
                    d(
                        "Max Connections",
                        "The biggest number of brain connections in the population",
                        [0.0, 0.0, 0.5],
                    ),
                ],
                Compress::Oldest,
            ),
            StreamGroup::new(
                "Age of Death Data",
                "The Age of death distribution of bibites in your simulation. \n*Using a rolling buffer of 1 hour*",
                age_curves(
                    "die",
                    "The age of death of the bibite that lived the longest",
                ),
                Compress::Oldest,
            ),
            StreamGroup::new(
                "Egg Laying Data",
                "The number of eggs laid by bibites in their lifetime.",
                vec![
                    d(
                        "Median eggs laid",
                        "Half of the bibites have laid at least this many eggs.",
                        [0.66, 0.66, 1.0],
                    ),
                    d(
                        "70th percentile eggs laid",
                        "Only 30% of the bibites have laid more eggs than this.",
                        [0.33, 0.33, 1.0],
                    ),
                    d(
                        "80th percentile eggs laid",
                        "Only 20% of the bibites have laid more eggs than this.",
                        BLUE,
                    ),
                    d(
                        "90th percentile eggs laid",
                        "Only 10% of the bibites have laid more eggs than this.",
                        [0.0, 0.0, 0.66],
                    ),
                    d(
                        "Max eggs laid",
                        "The most number of eggs laid by a bibite",
                        [0.0, 0.0, 0.33],
                    ),
                    d(
                        "Average eggs laid",
                        "The average number of eggs laid per bibite across your simulation",
                        WHITE,
                    ),
                ],
                Compress::Oldest,
            ),
            StreamGroup::new(
                "Birth/Death Data",
                "The birth/death numbers for bibites between each step",
                vec![
                    d(
                        "Bibite Birth",
                        "The number of bibite birth since the last point",
                        WHITE,
                    ),
                    d(
                        "Bibite Death",
                        "The number of bibite that died since the last point",
                        RED,
                    ),
                ],
                Compress::Sum,
            ),
        ];
        Self {
            groups,
            death_ages: RollingQuantile::new(3, 20, 0.05),
            last_births_total: 0,
            deaths_since_log: 0,
            hatches_since_log: 0,
            n_minutes: 0,
            genes: crate::genes_log::GenesLog::new(),
            progress: 0.0,
        }
    }

    /// `DataLogger.FixedUpdate` accumulator: returns `true` when a point is
    /// due (every [`LOG_PERIOD_SECONDS`] of simulated time).
    pub fn advance(&mut self, dt: f32) -> bool {
        self.progress += dt;
        if self.progress < LOG_PERIOD_SECONDS {
            return false;
        }
        self.progress -= LOG_PERIOD_SECONDS;
        true
    }

    /// `nMinutes++` of `DataLogger.FixedUpdate`: a minute has been logged. Returns `true` when an hour is full, which is when the genes
    /// stream logs a point (the minutes start over).
    pub fn count_minute(&mut self) -> bool {
        self.n_minutes += 1;
        if self.n_minutes >= 60 {
            self.n_minutes -= 60;
            true
        } else {
            false
        }
    }

    /// Logs one point per group (`s.Log()`); `points[i]` belongs to group `i`. The ages of death
    /// roll on to the next window (`GetAgeOfDeathData(peek: false)`).
    pub fn log(&mut self, points: &[Vec<f32>]) {
        for (group, point) in self.groups.iter_mut().zip(points) {
            group.history.push(point);
        }
        self.death_ages.increment_roll_counter();
    }

    /// `BibiteTracker.BibiteDeath`: a tracked bibite died at `age_hours`.
    pub fn record_death_age(&mut self, age_hours: f32) {
        self.death_ages.add(age_hours);
    }

    /// Whether any group holds a point (a world that has been running for a minute, or a save with history).
    #[must_use]
    pub fn has_history(&self) -> bool {
        self.groups
            .iter()
            .any(|group| group.history.n_data_at_scale(0) > 0)
    }

    /// The groups in the order of `DataLogger.saveableStack` (`data.bin`'s first block), with `None` for the genes stream: biomass,
    /// count, genes, brain size, age, age of death, eggs laid, births and deaths.
    const STACK: [Option<usize>; 8] = [
        Some(group::BIOMASS),
        Some(group::COUNT),
        None,
        Some(group::BRAIN_SIZE),
        Some(group::AGE),
        Some(group::DEATH_AGE),
        Some(group::EGGS_LAID),
        Some(group::BIRTH_DEATH),
    ];

    /// `DataLogger.SaveStateBin`: the progress to the next point (`f32`) and the minutes (`i32`), then each stream's bytes behind a
    /// length (`SaveableBinStack`).
    #[must_use]
    pub fn save_state_bin(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.progress.to_le_bytes());
        out.extend_from_slice(&self.n_minutes.to_le_bytes());
        for element in Self::STACK {
            let bytes = match element {
                Some(index) => self.groups[index].history.save_state_bin(),
                None => self.genes.save_state_bin(),
            };
            out.extend_from_slice(&(bytes.len() as i32).to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        out
    }

    /// `DataLogger.LoadStateBin` (game versions from 0.6.0a9 on, whose streams are in the format [`LogLikeParallel::load_state_bin`]
    /// reads): the progress, the minutes and every stream the bytes reach; one that does not fit is left empty.
    pub fn load_state_bin(&mut self, bytes: &[u8]) {
        if bytes.len() < 8 {
            return;
        }
        self.progress = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        self.n_minutes = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let mut at = 8;
        for element in Self::STACK {
            if at + 4 > bytes.len() {
                break;
            }
            let length =
                i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            let Ok(length) = usize::try_from(length) else {
                break;
            };
            let Some(payload) = bytes.get(at + 4..at + 4 + length) else {
                break;
            };
            match element {
                Some(index) => {
                    self.groups[index].history.load_state_bin(payload);
                }
                None => {
                    self.genes.load_state_bin(payload);
                }
            }
            at += 4 + length;
        }
    }

    /// `BibiteTracker.SaveStateBin`: the births and deaths since the last point (`i32` each), then the rolling estimator of the ages
    /// of death behind a length.
    #[must_use]
    pub fn save_tracker_bin(&mut self, births: i32, deaths: i32) -> Vec<u8> {
        let estimator = self.death_ages.save_state_bin();
        let mut out = Vec::with_capacity(12 + estimator.len());
        out.extend_from_slice(&births.to_le_bytes());
        out.extend_from_slice(&deaths.to_le_bytes());
        out.extend_from_slice(&(estimator.len() as i32).to_le_bytes());
        out.extend_from_slice(&estimator);
        out
    }

    /// `BibiteTracker.LoadStateBin` (versions from 0.6.0a9 on): the estimator of the ages of death is restored; returns the births and
    /// deaths since the last point.
    pub fn load_tracker_bin(&mut self, bytes: &[u8]) -> Option<(i32, i32)> {
        if bytes.len() < 12 {
            return None;
        }
        let births = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let deaths = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let length = i32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if let Ok(length) = usize::try_from(length)
            && let Some(payload) = bytes.get(12..12 + length)
        {
            self.death_ages.load_state_bin(payload);
        }
        Some((births, deaths))
    }

    /// `BibiteTracker.GetAgeOfDeathData`: the 25th, 50th, 75th percentiles and the maximum of the
    /// ages of death over the rolling hour (all zero before the first death).
    #[must_use]
    pub fn death_age_point(&self) -> Vec<f32> {
        DEATH_AGE_QUANTILES
            .iter()
            .map(|&p| self.death_ages.quantile(p))
            .collect()
    }
}

/// One tuple of the Greenwald-Khanna summary (`GKQuantileEstimator.Tuple`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct GkTuple {
    value: f32,
    gap: i32,
    delta: i32,
}

/// `float.CompareTo`: NaN sorts before every number.
fn compare_floats(a: f32, b: f32) -> i32 {
    if a < b {
        -1
    } else if a > b {
        1
    } else if a == b {
        0
    } else if a.is_nan() {
        if b.is_nan() { 0 } else { -1 }
    } else {
        1
    }
}

/// `GKQuantileEstimator<float>`: an epsilon-approximate quantile summary of a stream of values.
#[derive(Clone, Debug)]
pub struct GkQuantileEstimator {
    tuples: Vec<GkTuple>,
    compressing_interval: i32,
    epsilon: f32,
    n: i32,
}

impl GkQuantileEstimator {
    /// A summary with the given error bound (`new FloatGKQuantileEstimator(epsilon)`).
    #[must_use]
    pub fn new(epsilon: f32) -> Self {
        Self {
            tuples: Vec::new(),
            compressing_interval: ((1.0_f32 / (2.0_f32 * epsilon)).floor() as i32).max(1),
            epsilon,
            n: 0,
        }
    }

    /// `2 * epsilon * n`, rounded down: the width of the allowed uncertainty.
    fn band(&self) -> i32 {
        (2.0_f32 * self.epsilon * self.n as f32).floor() as i32
    }

    /// The index `value` goes in: `List<T>.BinarySearch` (an equal element's index when there is
    /// one), else where it would be inserted.
    fn insert_index(&self, value: f32) -> usize {
        let (mut lo, mut hi) = (0_i64, self.tuples.len() as i64 - 1);
        while lo <= hi {
            let mid = lo + ((hi - lo) >> 1);
            let order = compare_floats(self.tuples[mid as usize].value, value);
            if order == 0 {
                return mid as usize;
            }
            if order < 0 {
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        lo as usize
    }

    /// `Add`: inserts one value.
    pub fn add(&mut self, value: f32) {
        let mut tuple = GkTuple {
            value,
            gap: 1,
            delta: self.band(),
        };
        let index = self.insert_index(value);
        if index == 0 || index == self.tuples.len() {
            tuple.delta = 0;
        }
        self.tuples.insert(index, tuple);
        self.n += 1;
        if self.n % self.compressing_interval == 0 {
            self.compress();
        }
    }

    /// `Reset`: forgets every value.
    pub fn reset(&mut self) {
        self.tuples.clear();
        self.n = 0;
    }

    /// `GKQuantileEstimator.BytesSpace`: the bytes the summary takes as it is now (6, and 8 a tuple).
    #[must_use]
    pub fn bytes_space(&self) -> usize {
        6 + self.tuples.len() * 8
    }

    /// `GKQuantileEstimator.SaveStateBin`: the summary is compressed, then written as the count of values seen (`i32`), the tuple count
    /// (`i16`), the values (`f32`), the gaps and the deltas (`i16` each).
    #[must_use]
    pub fn save_state_bin(&mut self) -> Vec<u8> {
        self.compress();
        let count = self.tuples.len();
        let mut out = Vec::with_capacity(6 + count * 8);
        out.extend_from_slice(&self.n.to_le_bytes());
        out.extend_from_slice(&(count as i16).to_le_bytes());
        for tuple in &self.tuples {
            out.extend_from_slice(&tuple.value.to_le_bytes());
        }
        for tuple in &self.tuples {
            out.extend_from_slice(&(tuple.gap as i16).to_le_bytes());
        }
        for tuple in &self.tuples {
            out.extend_from_slice(&(tuple.delta as i16).to_le_bytes());
        }
        out
    }

    /// `GKQuantileEstimator.LoadStateBin`; `false` when the bytes are too short (the summary is then empty).
    pub fn load_state_bin(&mut self, bytes: &[u8]) -> bool {
        self.reset();
        if bytes.len() < 6 {
            return false;
        }
        let n = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let Ok(count) = usize::try_from(i16::from_le_bytes([bytes[4], bytes[5]])) else {
            return false;
        };
        if bytes.len() < 6 + count * 8 {
            return false;
        }
        let read_f32 = |at: usize| {
            f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let read_i16 = |at: usize| i32::from(i16::from_le_bytes([bytes[at], bytes[at + 1]]));
        for i in 0..count {
            self.tuples.push(GkTuple {
                value: read_f32(6 + i * 4),
                gap: read_i16(6 + count * 4 + i * 2),
                delta: read_i16(6 + count * 6 + i * 2),
            });
        }
        self.n = n;
        true
    }

    /// `GetQuantile`: the value at quantile `p` (0 when nothing was added).
    #[must_use]
    pub fn quantile(&self, p: f32) -> f32 {
        if self.tuples.is_empty() {
            return 0.0;
        }
        let target = p * (self.n - 1) as f32 + 1.0;
        let slack = (self.epsilon * self.n as f32).ceil() as i32;
        let mut best: Option<(usize, f32)> = None;
        let mut closest = (0_usize, f32::MAX);
        let mut rank = 0_i32;
        for (index, tuple) in self.tuples.iter().enumerate() {
            rank += tuple.gap;
            let max_rank = rank + tuple.delta;
            let distance = (target - (rank + max_rank) as f32 / 2.0).abs();
            if target - slack as f32 <= rank as f32
                && max_rank as f32 <= target + slack as f32
                && best.is_none_or(|(_, nearest)| distance < nearest)
            {
                best = Some((index, distance));
            }
            if distance < closest.1 {
                closest = (index, distance);
            }
        }
        // The game throws when no tuple fits; the summary's closest one is the best answer left.
        self.tuples[best.map_or(closest.0, |(index, _)| index)].value
    }

    /// `Compress`: merges the tuples whose uncertainty allows it.
    pub fn compress(&mut self) {
        let mut i = self.tuples.len() as i64 - 2;
        while i >= 1 {
            while (i as usize) + 1 < self.tuples.len() && self.delete_if_needed(i as usize) {}
            i -= 1;
        }
    }

    fn delete_if_needed(&mut self, i: usize) -> bool {
        let tuple = self.tuples[i];
        let mut next = self.tuples[i + 1];
        if tuple.delta >= next.delta && tuple.gap + next.gap + next.delta < self.band() {
            next.gap += tuple.gap;
            self.tuples[i + 1] = next;
            self.tuples.remove(i);
            true
        } else {
            false
        }
    }
}

/// `FloatRollingQuantileEstimator`: several [`GkQuantileEstimator`]s that all see every value but are
/// reset one after the other, so the oldest one always covers the last stretch of the stream.
#[derive(Clone, Debug)]
pub struct RollingQuantile {
    estimators: Vec<GkQuantileEstimator>,
    roll_period: i32,
    roll_counter: i32,
    oldest: usize,
}

impl RollingQuantile {
    /// `new FloatRollingQuantileEstimator(nEstimator, rollPeriod, epsilon)`.
    #[must_use]
    pub fn new(estimators: usize, roll_period: i32, epsilon: f32) -> Self {
        Self {
            estimators: (0..estimators.max(1))
                .map(|_| GkQuantileEstimator::new(epsilon))
                .collect(),
            roll_period,
            roll_counter: 0,
            oldest: 0,
        }
    }

    /// `Add`: every estimator takes the value.
    pub fn add(&mut self, value: f32) {
        for estimator in &mut self.estimators {
            estimator.add(value);
        }
    }

    /// `GetQuantile`: the oldest estimator's answer.
    #[must_use]
    pub fn quantile(&self, p: f32) -> f32 {
        self.estimators[self.oldest].quantile(p)
    }

    /// `RollingQuantileEstimator.SaveStateBin`: the roll counter (`i32`), then every estimator, the oldest first, as a length (`i32`)
    /// and its bytes. The game sizes each estimator's slot by its `BytesSpace` BEFORE the estimator compresses itself, so the
    /// compressed summary fills the start of a slot that is as large as the summary was and the rest of it stays zero.
    #[must_use]
    pub fn save_state_bin(&mut self) -> Vec<u8> {
        let count = self.estimators.len();
        let mut out = Vec::new();
        out.extend_from_slice(&self.roll_counter.to_le_bytes());
        for i in 0..count {
            let estimator = &mut self.estimators[(self.oldest + i) % count];
            let slot = estimator.bytes_space();
            let mut bytes = estimator.save_state_bin();
            bytes.resize(slot, 0);
            out.extend_from_slice(&(slot as i32).to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        out
    }

    /// `RollingQuantileEstimator.LoadStateBin`: the saved order becomes the physical one (the first is the oldest, as the game's
    /// `oldestEstimator` is not restored either); an estimator the bytes do not reach stays as it was.
    pub fn load_state_bin(&mut self, bytes: &[u8]) {
        if bytes.len() < 4 {
            return;
        }
        self.roll_counter = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        self.oldest = 0;
        let mut at = 4;
        for estimator in &mut self.estimators {
            if at + 4 > bytes.len() {
                break;
            }
            let length =
                i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            let Ok(length) = usize::try_from(length) else {
                break;
            };
            let Some(payload) = bytes.get(at + 4..at + 4 + length) else {
                break;
            };
            estimator.load_state_bin(payload);
            at += 4 + length;
        }
    }

    /// `IncrementRollCounter`: after `estimators * roll_period` calls the oldest estimator starts
    /// over and the next one becomes the oldest.
    pub fn increment_roll_counter(&mut self) {
        self.roll_counter += 1;
        if self.roll_counter >= self.estimators.len() as i32 * self.roll_period {
            self.roll_counter -= self.roll_period;
            self.estimators[self.oldest].reset();
            self.oldest = (self.oldest + 1) % self.estimators.len();
        }
    }
}

/// Y-axis maximum used by the line graphs (`DataStreamLinesGraph`):
/// `10^floor(log10 m) * ceil(m / 10^floor(log10 m))`, or `1` when `m == 0`.
#[must_use]
pub fn graph_axis_max(max_value: f32) -> f32 {
    if max_value <= 0.0 || !max_value.is_finite() {
        return 1.0;
    }
    let decade = 10.0_f32.powf(max_value.log10().floor());
    decade * (max_value / decade).ceil()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> LogLikeParallel {
        LogLikeParallel::new(
            &[
                LogLikeFormat::new(4, 3),
                LogLikeFormat::new(3, 2),
                LogLikeFormat::new(2, 0),
            ],
            1,
            Compress::Oldest,
        )
    }

    #[test]
    fn newest_first_and_ring_wraps() {
        let mut log = tiny();
        for v in 1..=6 {
            log.push(&[v as f32]);
        }
        assert_eq!(log.n_data_at_scale(0), 4);
        assert_eq!(log.get(0, 0), &[6.0]);
        assert_eq!(log.get(0, 3), &[3.0]);
    }

    #[test]
    fn every_feed_ratio_pushes_the_oldest_of_the_block_up() {
        let mut log = tiny();
        for v in 1..=3 {
            log.push(&[v as f32]);
        }
        // The third push completes a block of 3: oldest = 1.
        assert_eq!(log.n_data_at_scale(1), 1);
        assert_eq!(log.get(1, 0), &[1.0]);
        for v in 4..=6 {
            log.push(&[v as f32]);
        }
        assert_eq!(log.get(1, 0), &[4.0]);
        assert_eq!(log.get(1, 1), &[1.0]);
    }

    #[test]
    fn sum_compression_adds_the_block() {
        let mut log = LogLikeParallel::new(
            &[LogLikeFormat::new(4, 2), LogLikeFormat::new(2, 0)],
            2,
            Compress::Sum,
        );
        log.push(&[1.0, 10.0]);
        log.push(&[2.0, 20.0]);
        assert_eq!(log.get(1, 0), &[3.0, 30.0]);
    }

    #[test]
    fn unbounded_last_scale_grows() {
        let mut log = LogLikeParallel::new(
            &[
                LogLikeFormat::new(2, 2),
                LogLikeFormat::new(2, 2),
                LogLikeFormat::new(0, 0),
            ],
            1,
            Compress::Oldest,
        );
        for v in 1..=8 {
            log.push(&[v as f32]);
        }
        assert!(log.scale_has_data(2));
        assert_eq!(log.n_data_at_scale(2), 2);
        assert_eq!(log.scale_step(2), 4);
    }

    #[test]
    fn logger_fires_every_minute() {
        let mut logger = DataLogger::new();
        let mut due = 0;
        for _ in 0..(60 * 30 * 3) {
            if logger.advance(1.0 / 30.0) {
                due += 1;
            }
        }
        assert!((2..=3).contains(&due), "{due}");
    }

    #[test]
    fn the_groups_are_the_games_in_the_games_order_with_their_texts() {
        let logger = DataLogger::new();
        let titles: Vec<&str> = logger.groups.iter().map(|group| group.title).collect();
        assert_eq!(
            titles,
            [
                "Biomass Data",
                "Entities Count Data",
                "Age Data",
                "Brain Size Data",
                "Age of Death Data",
                "Egg Laying Data",
                "Birth/Death Data"
            ]
        );
        assert_eq!(logger.groups[group::DEATH_AGE].streams.len(), 4);
        assert_eq!(logger.groups[group::EGGS_LAID].streams.len(), 6);
        assert_eq!(logger.groups[group::BIRTH_DEATH].streams.len(), 2);
        for group in &logger.groups {
            assert!(!group.description.is_empty(), "{}", group.title);
            for stream in &group.streams {
                assert!(!stream.description.is_empty(), "{}", stream.label);
            }
        }
        // The two age groups differ in what their curves describe.
        let alive = &logger.groups[group::AGE].streams;
        let dead = &logger.groups[group::DEATH_AGE].streams;
        assert_eq!(alive[1].label, dead[1].label);
        assert!(alive[1].description.contains("are younger"));
        assert!(dead[1].description.contains("die younger"));
        assert!(
            logger.groups[group::DEATH_AGE]
                .description
                .contains("rolling buffer of 1 hour")
        );
    }

    #[test]
    fn a_quantile_summary_answers_within_its_error_bound() {
        let mut summary = GkQuantileEstimator::new(0.05);
        assert_eq!(summary.quantile(0.5), 0.0, "nothing added yet");
        // 1..=200 in a scrambled order.
        let mut value = 7_u32;
        for _ in 0..200 {
            value = (value * 41 + 11) % 200;
            summary.add(value as f32 + 1.0);
        }
        let n = 200.0_f32;
        for p in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            let answer = summary.quantile(p);
            let rank = (answer - 1.0) / (n - 1.0);
            assert!((rank - p).abs() <= 0.06, "p {p}: {answer} has rank {rank}");
        }
        assert!(summary.tuples.len() < 200, "the summary is compressed");
        summary.reset();
        assert_eq!(summary.quantile(0.5), 0.0);
    }

    #[test]
    fn the_rolling_estimator_forgets_the_oldest_window() {
        // 2 estimators, a roll every 3 calls: the first reset comes after 2 * 3 calls.
        let mut rolling = RollingQuantile::new(2, 3, 0.05);
        for _ in 0..6 {
            rolling.add(10.0);
            rolling.increment_roll_counter();
        }
        // The oldest (0) was reset at the 6th call; estimator 1 still holds all six values.
        assert_eq!(rolling.quantile(0.5), 10.0);
        for _ in 0..3 {
            rolling.add(20.0);
            rolling.increment_roll_counter();
        }
        // The oldest is now the estimator that was reset at call 6: it only saw the 20s.
        assert_eq!(rolling.quantile(0.0), 20.0);
        assert_eq!(rolling.quantile(1.0), 20.0);
    }

    #[test]
    fn deaths_feed_the_death_age_point_and_logging_rolls_the_window() {
        let mut logger = DataLogger::new();
        assert_eq!(logger.death_age_point(), vec![0.0; 4]);
        for age in [1.0, 2.0, 3.0, 4.0, 5.0] {
            logger.record_death_age(age);
        }
        let point = logger.death_age_point();
        assert_eq!(point.len(), 4);
        assert_eq!(point[3], 5.0, "the maximum");
        assert!(point[0] <= point[1] && point[1] <= point[2] && point[2] <= point[3]);
        // One log per minute rolls the window: after 100 minutes the estimator that answers was
        // reset at minute 60 and has seen none of those deaths.
        let none: Vec<Vec<f32>> = Vec::new();
        for _ in 0..59 {
            logger.log(&none);
        }
        assert_eq!(
            logger.death_age_point()[3],
            5.0,
            "still inside the first hour"
        );
        for _ in 59..100 {
            logger.log(&none);
        }
        assert_eq!(logger.death_age_point(), vec![0.0; 4]);
        logger.record_death_age(9.0);
        assert_eq!(logger.death_age_point()[3], 9.0);
    }

    /// A log of two streams over three scales (4, 3 and 2 points), `pushes` points pushed.
    fn pair_log(pushes: usize) -> LogLikeParallel {
        let mut log = LogLikeParallel::new(
            &[
                LogLikeFormat::new(4, 3),
                LogLikeFormat::new(3, 2),
                LogLikeFormat::new(2, 0),
            ],
            2,
            Compress::Sum,
        );
        for v in 1..=pushes {
            log.push(&[v as f32, 100.0 + v as f32]);
        }
        log
    }

    #[test]
    fn a_saved_log_reads_back_as_the_same_points_and_goes_on_the_same_way() {
        // 40 pushes wrap every ring, so the saved points come from the middle of the rings while the loaded ones start at the
        // first slots; both must show the same points and take the next pushes alike.
        let mut original = pair_log(40);
        let bytes = original.save_state_bin();
        let points = original.all_points().len();
        assert_eq!(
            i32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
            points
        );
        assert_eq!((bytes[4], bytes[5]), (2, 3), "the width and the scales");
        assert_eq!(bytes.len(), 6 + points * 2 * 4 + 2 + 3);
        // The floats go stream after stream, the newest point of the finest scale first.
        let float = |index: usize| {
            f32::from_le_bytes(bytes[6 + index * 4..10 + index * 4].try_into().unwrap())
        };
        assert_eq!(float(0), original.get(0, 0)[0]);
        assert_eq!(float(points), original.get(0, 0)[1]);
        assert_eq!(float(1), original.get(0, 1)[0]);

        let mut loaded = pair_log(0);
        assert!(loaded.load_state_bin(&bytes));
        assert_eq!(loaded.all_points(), original.all_points());
        for scale in 0..3 {
            assert_eq!(
                loaded.n_data_at_scale(scale),
                original.n_data_at_scale(scale)
            );
        }
        assert_eq!(
            loaded.save_state_bin(),
            bytes,
            "saved again, the same bytes"
        );
        for v in 41..=70 {
            let point = [v as f32, 100.0 + v as f32];
            original.push(&point);
            loaded.push(&point);
            assert_eq!(loaded.all_points(), original.all_points(), "push {v}");
        }
    }

    #[test]
    fn the_games_own_shape_of_log_reads_back_through_every_scale() {
        // The Graphs panel's six scales: 700 pushes fill the first two and reach the third and the fourth (23 and 3 points).
        let make = |pushes: usize| {
            let mut log = LogLikeParallel::new(&PARALLEL_DETAILED, 3, Compress::Oldest);
            for v in 0..pushes {
                log.push(&[v as f32, (v * 2) as f32, 0.5]);
            }
            log
        };
        let original = make(700);
        let counts: Vec<usize> = (0..6)
            .map(|scale| original.n_data_at_scale(scale))
            .collect();
        assert_eq!(counts, [60, 60, 23, 3, 0, 0]);
        let bytes = original.save_state_bin();
        let mut loaded = make(0);
        assert!(loaded.load_state_bin(&bytes));
        assert_eq!(loaded.all_points(), original.all_points());
        assert_eq!(loaded.save_state_bin(), bytes);
        // The coarser scales keep their meaning: the next block of ten completes at the same push in both.
        let (mut original, mut loaded) = (original, loaded);
        for v in 700..760 {
            let point = [v as f32, (v * 2) as f32, 0.5];
            original.push(&point);
            loaded.push(&point);
        }
        assert_eq!(loaded.all_points(), original.all_points());
    }

    #[test]
    fn bytes_that_do_not_fit_the_log_change_nothing() {
        let bytes = pair_log(10).save_state_bin();
        let mut log = pair_log(5);
        let before = log.all_points();
        assert!(!log.load_state_bin(&[1, 2, 3]), "not even a header");
        assert!(!log.load_state_bin(&bytes[..bytes.len() - 1]), "cut short");
        let mut wide = LogLikeParallel::new(
            &[
                LogLikeFormat::new(4, 3),
                LogLikeFormat::new(3, 2),
                LogLikeFormat::new(2, 0),
            ],
            3,
            Compress::Sum,
        );
        assert!(!wide.load_state_bin(&bytes), "another width");
        let mut short = LogLikeParallel::new(
            &[LogLikeFormat::new(4, 3), LogLikeFormat::new(3, 0)],
            2,
            Compress::Sum,
        );
        assert!(!short.load_state_bin(&bytes), "another number of scales");
        assert_eq!(log.all_points(), before);
        assert_eq!(wide.all_points(), Vec::<Vec<f32>>::new());
    }

    #[test]
    fn a_quantile_summary_and_a_rolling_estimator_read_back_as_they_were() {
        let mut summary = GkQuantileEstimator::new(0.05);
        let mut value = 7_u32;
        for _ in 0..200 {
            value = (value * 41 + 11) % 200;
            summary.add(value as f32 + 1.0);
        }
        let bytes = summary.save_state_bin();
        let count = summary.tuples.len();
        assert_eq!(i32::from_le_bytes(bytes[..4].try_into().unwrap()), 200);
        assert_eq!(
            i16::from_le_bytes(bytes[4..6].try_into().unwrap()) as usize,
            count
        );
        assert_eq!(
            bytes.len(),
            6 + count * 8,
            "a value, a gap and a delta each"
        );
        let mut loaded = GkQuantileEstimator::new(0.05);
        assert!(loaded.load_state_bin(&bytes));
        assert_eq!(loaded.n, summary.n);
        assert_eq!(loaded.tuples, summary.tuples);
        for p in [0.0_f32, 0.1, 0.5, 0.9, 1.0] {
            assert_eq!(loaded.quantile(p), summary.quantile(p), "p {p}");
        }
        assert_eq!(
            loaded.save_state_bin(),
            bytes,
            "saved again, the same bytes"
        );
        assert!(!loaded.load_state_bin(&bytes[..5]), "no header");
        assert_eq!(
            loaded.quantile(0.5),
            0.0,
            "a summary that failed to load is empty"
        );

        // Three windows of four logs: after 30 logs the oldest is the third, which the saved order puts first.
        let mut rolling = RollingQuantile::new(3, 4, 0.05);
        for round in 0..30 {
            rolling.add(round as f32);
            rolling.increment_roll_counter();
        }
        let bytes = rolling.save_state_bin();
        assert_eq!(
            i32::from_le_bytes(bytes[..4].try_into().unwrap()),
            10,
            "the roll counter"
        );
        let mut restored = RollingQuantile::new(3, 4, 0.05);
        restored.load_state_bin(&bytes);
        for p in [0.0_f32, 0.5, 1.0] {
            assert_eq!(restored.quantile(p), rolling.quantile(p), "p {p}");
        }
        assert_eq!(restored.save_state_bin(), bytes);
        // The game sizes each estimator's slot before the estimator compresses itself: a summary with tuples waiting to be merged is
        // written at the start of a slot as large as the summary was, and the rest of the slot is zero.
        let mut pending = RollingQuantile::new(2, 4, 0.05);
        let mut value = 3_u32;
        for _ in 0..137 {
            value = (value * 41 + 11) % 200;
            pending.add(value as f32);
        }
        let slots: Vec<usize> = pending
            .estimators
            .iter()
            .map(GkQuantileEstimator::bytes_space)
            .collect();
        let compressed: Vec<usize> = pending
            .estimators
            .clone()
            .iter_mut()
            .map(|estimator| estimator.save_state_bin().len())
            .collect();
        assert!(
            compressed[0] < slots[0],
            "tuples were waiting: {compressed:?} {slots:?}"
        );
        let bytes = pending.save_state_bin();
        assert_eq!(
            bytes.len(),
            4 + slots.iter().map(|slot| 4 + slot).sum::<usize>()
        );
        assert_eq!(
            i32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
            slots[0]
        );
        let first = &bytes[8..8 + slots[0]];
        assert!(
            first[compressed[0]..].iter().all(|byte| *byte == 0),
            "zero padding"
        );
        // It reads back as the compressed summary, and saved again it fills the slot it needs.
        let mut back = RollingQuantile::new(2, 4, 0.05);
        back.load_state_bin(&bytes);
        assert_eq!(back.estimators[0].tuples.len(), (compressed[0] - 6) / 8);
        assert_eq!(
            back.estimators[0].quantile(0.5),
            pending.estimators[0].quantile(0.5)
        );
        assert_eq!(
            back.save_state_bin().len(),
            4 + 2 * 4 + compressed[0] + compressed[1]
        );
        // Both forget the same window at the same log afterwards.
        for round in 30..70 {
            rolling.add(round as f32);
            rolling.increment_roll_counter();
            restored.add(round as f32);
            restored.increment_roll_counter();
            assert_eq!(restored.quantile(0.5), rolling.quantile(0.5), "log {round}");
        }
    }

    #[test]
    fn the_logger_and_tracker_blocks_read_back_and_keep_the_genes_stream() {
        let mut logger = DataLogger::new();
        assert!(!logger.has_history());
        // A genes stream with points of its own.
        for hour in 1..=12 {
            let genes: Vec<[f32; crate::genome::N_GENES]> = (0..hour)
                .map(|bibite| {
                    let mut genes = [0.0_f32; crate::genome::N_GENES];
                    for (slot, def) in genes.iter_mut().zip(crate::genome::GENE_DEFS.iter()) {
                        *slot = def.min + def.span() * ((bibite * 5 + hour) % 11) as f32 / 10.0;
                    }
                    genes
                })
                .collect();
            logger
                .genes
                .push(crate::genes_log::GenesBuckets::from_genes(&genes));
        }
        for minute in 1..=70 {
            let points: Vec<Vec<f32>> = logger
                .groups
                .iter()
                .map(|group| {
                    (0..group.streams.len())
                        .map(|stream| (minute * 10 + stream) as f32)
                        .collect()
                })
                .collect();
            logger.log(&points);
        }
        assert!(logger.has_history());
        for age in [1.5, 2.5, 8.0] {
            logger.record_death_age(age);
        }
        logger.progress = 12.5;
        logger.n_minutes = 70;
        let block = logger.save_state_bin();
        let tracker = logger.save_tracker_bin(3, 2);

        let mut loaded = DataLogger::new();
        loaded.load_state_bin(&block);
        assert_eq!(loaded.load_tracker_bin(&tracker), Some((3, 2)));
        assert_eq!((loaded.progress, loaded.n_minutes), (12.5, 70));
        assert_eq!(loaded.genes.n_data(), logger.genes.n_data());
        // 12 hours: the 6 newest at full resolution and 2 made of the 6 before them.
        assert_eq!(loaded.genes.n_data(), 8);
        for i in 0..loaded.genes.n_data() {
            assert_eq!(loaded.genes.get(i), logger.genes.get(i), "genes point {i}");
        }
        for (a, b) in loaded.groups.iter().zip(&logger.groups) {
            assert_eq!(
                a.history.all_points(),
                b.history.all_points(),
                "{}",
                a.title
            );
            assert_eq!(a.history.n_data_at_scale(1), b.history.n_data_at_scale(1));
        }
        assert_eq!(loaded.death_age_point(), logger.death_age_point());
        assert_eq!(
            loaded.save_state_bin(),
            block,
            "saved again, the same bytes"
        );
        assert_eq!(loaded.save_tracker_bin(3, 2), tracker);
        // Truncated blocks load what they reach and nothing more.
        let mut short = DataLogger::new();
        short.load_state_bin(&block[..block.len() / 3]);
        assert_eq!(short.n_minutes, 70);
        assert!(short.groups[group::BIOMASS].history.n_data_at_scale(0) > 0);
        assert_eq!(
            short.groups[group::BIRTH_DEATH].history.n_data_at_scale(0),
            0
        );
        assert_eq!(short.load_tracker_bin(&[0; 5]), None);
    }

    #[test]
    fn axis_max_rounds_up_to_one_significant_digit() {
        assert_eq!(graph_axis_max(0.0), 1.0);
        assert_eq!(graph_axis_max(3.2), 4.0);
        assert_eq!(graph_axis_max(180.0), 200.0);
        assert_eq!(graph_axis_max(1000.0), 1000.0);
    }
}
