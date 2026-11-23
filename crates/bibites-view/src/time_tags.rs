//! RUSTibites Improved: time tags. A tag is a moment of the world's simulated time with a name the player gave it ("drought starts", "predators in"). One
//! list serves every panel that has a time axis: the Stats and Analysis panel's data graphs and gene analysis page, and the Ancestral Lineage Graph (tree and
//! mesh); a tag set in one shows in all of them, and each panel has its own switch to hide them (the switches are the player's, kept in
//! `time_tag_view.json`).
//!
//! The tags are NOT part of a save (saves stay in the game's format): they are kept in the state folder under the name of the save the world was saved or
//! loaded as (`time_tags/<name>.json`), see `App::time_tags_after_save` / `time_tags_after_load`.
//!
//! This module is the model and the maths that put a time on a panel's axis. The drawing is `time_tags_ui.rs`.

use bibites_core::datalog::LogLikeFormat;
use bibites_core::species_log::{ACQUISITION_MINUTES, SPECIES_FORMATS};
use bibites_io::storage::{TimeTagRecord, TimeTagsFile};

use crate::ui::Rgba;

/// The longest name of a tag, in characters.
pub const MAX_LABEL_CHARS: usize = 32;
/// The most tags a world keeps (a hundred hit areas of removal per panel, and a graph with more would be unreadable anyway).
pub const MAX_TAGS: usize = 100;

/// The panels that show time tags; each has its own switch and its own name field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TagPanel {
    /// The Stats and Analysis panel's data graphs page.
    Graphs,
    /// The Stats and Analysis panel's gene analysis page.
    Genes,
    /// The Ancestral Lineage Graph (the tree and the mesh view).
    Lineage,
}

impl TagPanel {
    /// The first hit id of the panel's time tag controls (the Tag time button, the show / hide button, the name field, and `+ 3` the area of the panel's
    /// graph that takes the click while the name field is open) and, `remove_base`, of its tags' remove buttons: the blocks 9600.. (controls) and 9700..
    /// (removal) of `ui::UiId`'s table.
    #[must_use]
    pub const fn hit_base(self) -> u32 {
        match self {
            Self::Graphs => 9600,
            Self::Genes => 9610,
            Self::Lineage => 9620,
        }
    }

    /// The first hit id of the panel's tags' remove buttons.
    #[must_use]
    pub const fn remove_base(self) -> u32 {
        match self {
            Self::Graphs => 9700,
            Self::Genes => 9800,
            Self::Lineage => 9900,
        }
    }

    /// The panel's name in a status line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Graphs => "the data graphs",
            Self::Genes => "the gene analysis",
            Self::Lineage => "the lineage graph",
        }
    }
}

/// One tag.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeTag {
    /// Its number in its world (never reused, so a removed tag's colour is not handed to the next one).
    pub id: u32,
    /// The world's simulated time in seconds when it was set.
    pub seconds: f64,
    /// Its name.
    pub label: String,
}

/// The colours the tags take, by their number (bright, apart from each other and from the graphs' own curves and the lineage's navy).
const PALETTE: [Rgba; 6] = [
    [255, 196, 0, 255],
    [0, 214, 255, 255],
    [255, 96, 150, 255],
    [130, 255, 110, 255],
    [200, 150, 255, 255],
    [255, 140, 50, 255],
];

impl TimeTag {
    /// The tag's colour.
    #[must_use]
    pub fn color(&self) -> Rgba {
        PALETTE[self.id as usize % PALETTE.len()]
    }
}

/// The time tags of one world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimeTags {
    tags: Vec<TimeTag>,
    next_id: u32,
}

impl TimeTags {
    /// No tags.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The tags, oldest first.
    #[cfg(test)]
    #[must_use]
    pub fn all(&self) -> &[TimeTag] {
        &self.tags
    }

    /// How many tags.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// Sets a tag at `seconds` named `label` (control characters dropped, cut to [`MAX_LABEL_CHARS`]; a blank name becomes "Tag N"); its number, or `None`
    /// when the world already has [`MAX_TAGS`] or the time is not a number.
    pub fn add(&mut self, seconds: f64, label: &str) -> Option<u32> {
        if self.tags.len() >= MAX_TAGS || !seconds.is_finite() {
            return None;
        }
        self.next_id += 1;
        let id = self.next_id;
        let cleaned: String = label.chars().filter(|c| !c.is_control()).collect();
        let cleaned: String = cleaned.trim().chars().take(MAX_LABEL_CHARS).collect();
        let label = if cleaned.is_empty() { format!("Tag {id}") } else { cleaned };
        self.tags.push(TimeTag { id, seconds, label });
        self.tags.sort_by(|a, b| a.seconds.total_cmp(&b.seconds).then(a.id.cmp(&b.id)));
        Some(id)
    }

    /// Removes tag `id`; whether there was one.
    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.tags.len();
        self.tags.retain(|tag| tag.id != id);
        self.tags.len() != before
    }

    /// The tag `id`.
    #[must_use]
    pub fn get(&self, id: u32) -> Option<&TimeTag> {
        self.tags.iter().find(|tag| tag.id == id)
    }

    /// The tags that are not in the future of a world whose clock reads `now` seconds.
    pub fn up_to(&self, now: f64) -> impl Iterator<Item = &TimeTag> {
        self.tags.iter().filter(move |tag| tag.seconds <= now)
    }

    /// The tags as stored.
    #[must_use]
    pub fn to_file(&self) -> TimeTagsFile {
        TimeTagsFile {
            tags: self
                .tags
                .iter()
                .map(|tag| TimeTagRecord { id: tag.id, seconds: tag.seconds, label: tag.label.clone() })
                .collect(),
        }
    }

    /// The tags of a stored file for a world whose clock reads `now` seconds: a tag from the future of this world (it belongs to a timeline this save does
    /// not hold: the player went back to an older save), one that is not a number, and a repeated number are left out; the names are cleaned as
    /// [`TimeTags::add`] does.
    #[must_use]
    pub fn from_file(file: &TimeTagsFile, now: f64) -> Self {
        let mut tags = Self::new();
        let mut seen = std::collections::HashSet::new();
        for record in &file.tags {
            if !record.seconds.is_finite() || record.seconds > now || record.seconds < 0.0 || !seen.insert(record.id) {
                continue;
            }
            let cleaned: String = record.label.chars().filter(|c| !c.is_control()).collect();
            let cleaned: String = cleaned.trim().chars().take(MAX_LABEL_CHARS).collect();
            let label = if cleaned.is_empty() { format!("Tag {}", record.id) } else { cleaned };
            tags.next_id = tags.next_id.max(record.id);
            tags.tags.push(TimeTag { id: record.id, seconds: record.seconds, label });
        }
        tags.tags.truncate(MAX_TAGS);
        tags.tags.sort_by(|a, b| a.seconds.total_cmp(&b.seconds).then(a.id.cmp(&b.id)));
        tags
    }
}

/// How many simulated minutes ago `tag_seconds` was, for a clock at `now_seconds` (never negative).
#[must_use]
pub fn minutes_ago(now_seconds: f64, tag_seconds: f64) -> f64 {
    ((now_seconds - tag_seconds) / 60.0).max(0.0)
}

/// Where a tag `minutes` old stands along a data graph whose plot spans `span_minutes` (the grid's own labels: the left edge is the whole span ago, the
/// right edge is now): `0.0` at the left edge, `1.0` at the right one, `None` when the tag is older than the span.
#[must_use]
pub fn graph_fraction(minutes: f64, span_minutes: f64) -> Option<f32> {
    if span_minutes <= 0.0 || !minutes.is_finite() || minutes > span_minutes {
        return None;
    }
    Some((1.0 - minutes / span_minutes) as f32)
}

/// The place of a moment `minutes` ago on the species log's point scale (`TimeOfPointInSerialConfig` the other way round): `-1.0` is now, `0.0` the newest
/// point, and between two points the place is fractional. The scales hold 12, 12, 18 ... points whose steps grow by their feed ratios, so a long time ago
/// is only a few points further.
#[must_use]
pub fn species_point_of_minutes(minutes: f64) -> f64 {
    serial_place(&SPECIES_FORMATS, f64::from(ACQUISITION_MINUTES), minutes)
}

/// The inverse of [`serial_place`]: how many time units ago `place` is on the point scale of a log with these `formats` whose newest points are `step` units
/// apart (`-1.0` and anything before it is now, whole numbers are the points, and between two points the time is in proportion). `None` when the place is
/// past the oldest point of a log of finite scales. Clicking a moment on a panel's axis turns into a time with this.
#[must_use]
pub fn serial_ago(formats: &[LogLikeFormat], step: f64, place: f64) -> Option<f64> {
    let mut step = step;
    let mut remaining = place + 1.0;
    if remaining.is_nan() || remaining <= 0.0 {
        return Some(0.0);
    }
    let mut ago = 0.0_f64;
    for (index, format) in formats.iter().enumerate() {
        if index + 1 == formats.len() && format.size == 0 {
            return Some(ago + remaining * step);
        }
        let size = format.size as f64;
        if remaining <= size {
            return Some(ago + remaining * step);
        }
        ago += step * size;
        remaining -= size;
        step *= format.feed_ratio as f64;
    }
    None
}

/// The inverse of [`species_point_of_minutes`]: how many minutes ago a place on the species log's point scale is (`None` when it is past the log).
#[must_use]
pub fn species_minutes_of_place(place: f64) -> Option<f64> {
    serial_ago(&SPECIES_FORMATS, f64::from(ACQUISITION_MINUTES), place)
}

/// The place of a moment `ago` time units back on the point scale of a log with these `formats` whose newest points are `step` units apart (the log's
/// acquisition step): `-1.0` is now, `0.0` the newest point, whole numbers the points, and between two points a fraction. A last format of size 0 holds
/// any number of points; when the formats are all finite and `ago` is older than their oldest point the place is infinite (it is not on the scale).
#[must_use]
pub fn serial_place(formats: &[LogLikeFormat], step: f64, ago: f64) -> f64 {
    let mut step = step;
    let mut remaining = ago.max(0.0);
    let mut before = 0.0_f64;
    for (index, format) in formats.iter().enumerate() {
        if index + 1 == formats.len() && format.size == 0 {
            return before + remaining / step - 1.0;
        }
        let size = format.size as f64;
        let span = step * size;
        if remaining <= span {
            return before + remaining / step - 1.0;
        }
        remaining -= span;
        before += size;
        step *= format.feed_ratio as f64;
    }
    f64::INFINITY
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::species_log::minutes_of_point;

    #[test]
    fn a_tag_gets_a_number_a_clean_name_and_the_list_stays_in_time_order() {
        let mut tags = TimeTags::new();
        let later = tags.add(7200.0, "  Predators in \n").unwrap();
        let earlier = tags.add(3600.0, "").unwrap();
        assert_eq!((later, earlier), (1, 2));
        assert_eq!(tags.all().iter().map(|tag| tag.label.as_str()).collect::<Vec<_>>(), ["Tag 2", "Predators in"]);
        assert_eq!(tags.all().iter().map(|tag| tag.seconds).collect::<Vec<_>>(), [3600.0, 7200.0]);
        let long = tags.add(1.0, &"x".repeat(100)).unwrap();
        assert_eq!(tags.get(long).unwrap().label.chars().count(), MAX_LABEL_CHARS);
        assert!(tags.add(f64::NAN, "nan").is_none());
        assert_eq!(tags.len(), 3);
    }

    #[test]
    fn a_removed_tags_number_is_not_given_again_and_the_world_keeps_at_most_max_tags() {
        let mut tags = TimeTags::new();
        let first = tags.add(1.0, "a").unwrap();
        assert!(tags.remove(first));
        assert!(!tags.remove(first));
        assert_eq!(tags.add(2.0, "b"), Some(2));
        for n in 0..MAX_TAGS {
            tags.add(10.0 + n as f64, "x");
        }
        assert_eq!(tags.len(), MAX_TAGS);
        assert!(tags.add(500.0, "one too many").is_none());
    }

    #[test]
    fn colours_follow_the_number_and_neighbours_differ() {
        let mut tags = TimeTags::new();
        let ids: Vec<u32> = (0..7).map(|n| tags.add(f64::from(n), "t").unwrap()).collect();
        let colours: Vec<_> = ids.iter().map(|id| tags.get(*id).unwrap().color()).collect();
        for pair in colours.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
        assert_eq!(colours[0], colours[6], "the palette wraps after its six colours");
    }

    #[test]
    fn only_the_tags_not_in_the_future_show() {
        let mut tags = TimeTags::new();
        tags.add(100.0, "past");
        tags.add(500.0, "future");
        let shown: Vec<_> = tags.up_to(300.0).map(|tag| tag.label.as_str()).collect();
        assert_eq!(shown, ["past"]);
        assert_eq!(tags.up_to(500.0).count(), 2, "a tag set this very second shows");
    }

    #[test]
    fn a_stored_file_comes_back_without_the_tags_of_another_timeline_or_broken_ones() {
        let file = TimeTagsFile {
            tags: vec![
                TimeTagRecord { id: 3, seconds: 100.0, label: "ok".to_owned() },
                TimeTagRecord { id: 4, seconds: 9_999.0, label: "after this save".to_owned() },
                TimeTagRecord { id: 5, seconds: f64::NAN, label: "nan".to_owned() },
                TimeTagRecord { id: 3, seconds: 150.0, label: "repeated number".to_owned() },
                TimeTagRecord { id: 6, seconds: -5.0, label: "before the world".to_owned() },
                TimeTagRecord { id: 7, seconds: 50.0, label: "  \t ".to_owned() },
            ],
        };
        let tags = TimeTags::from_file(&file, 1_000.0);
        assert_eq!(tags.all().iter().map(|tag| (tag.id, tag.label.as_str())).collect::<Vec<_>>(), [(7, "Tag 7"), (3, "ok")]);
        // The next tag is numbered after every number the file had used that survived.
        let mut tags = tags;
        assert_eq!(tags.add(900.0, "next"), Some(8));
        // The round trip keeps everything.
        let again = TimeTags::from_file(&tags.to_file(), 1_000.0);
        assert_eq!(again.all(), tags.all());
    }

    #[test]
    fn a_tags_age_is_in_minutes_and_never_negative() {
        assert_eq!(minutes_ago(3600.0, 0.0), 60.0);
        assert_eq!(minutes_ago(100.0, 400.0), 0.0);
    }

    #[test]
    fn the_graphs_x_is_the_whole_span_ago_at_the_left_edge_and_now_at_the_right() {
        assert_eq!(graph_fraction(0.0, 60.0), Some(1.0));
        assert_eq!(graph_fraction(60.0, 60.0), Some(0.0));
        assert_eq!(graph_fraction(15.0, 60.0), Some(0.75));
        assert_eq!(graph_fraction(61.0, 60.0), None, "older than the plot");
        assert_eq!(graph_fraction(1.0, 0.0), None);
    }

    #[test]
    fn the_genes_logs_hours_map_onto_its_points_and_a_finite_log_ends() {
        use bibites_core::genes_log::SERIAL_HEAVY;
        // Hourly points, the first six one hour apart, the next ones three hours apart.
        assert_eq!(serial_place(&SERIAL_HEAVY, 1.0, 0.0), -1.0);
        assert_eq!(serial_place(&SERIAL_HEAVY, 1.0, 1.0), 0.0);
        assert_eq!(serial_place(&SERIAL_HEAVY, 1.0, 6.0), 5.0);
        assert_eq!(serial_place(&SERIAL_HEAVY, 1.0, 9.0), 6.0);
        assert!((serial_place(&SERIAL_HEAVY, 1.0, 7.5) - 5.5).abs() < 1e-9);
        // A log of finite scales only: a moment older than its oldest point is not on it.
        let finite = [LogLikeFormat::new(2, 2), LogLikeFormat::new(2, 2)];
        assert_eq!(serial_place(&finite, 1.0, 2.0), 1.0);
        assert_eq!(serial_place(&finite, 1.0, 6.0), 3.0);
        assert!(serial_place(&finite, 1.0, 6.5).is_infinite());
    }

    #[test]
    fn a_place_on_a_point_scale_is_turned_back_into_the_time_it_came_from() {
        use bibites_core::genes_log::SERIAL_HEAVY;
        // Every time makes the same time again through its place, on the genes log's scale and on the species log's.
        for hours in [0.0, 0.25, 1.0, 5.5, 6.0, 7.5, 9.0, 40.0, 400.0] {
            let place = serial_place(&SERIAL_HEAVY, 1.0, hours);
            let back = serial_ago(&SERIAL_HEAVY, 1.0, place).unwrap();
            assert!((back - hours).abs() < 1e-9, "{hours} h came back as {back}");
        }
        for minutes in [0.0, 1.0, 17.0, 59.5, 120.0, 1_000.0, 5_000.0, 100_000.0] {
            let back = species_minutes_of_place(species_point_of_minutes(minutes)).unwrap();
            assert!((back - minutes).abs() < 1e-6 * minutes.max(1.0), "{minutes} minutes came back as {back}");
        }
        // Now, and anything before it, is no time ago.
        assert_eq!(serial_ago(&SERIAL_HEAVY, 1.0, -1.0), Some(0.0));
        assert_eq!(serial_ago(&SERIAL_HEAVY, 1.0, -7.0), Some(0.0));
        // A log of finite scales ends: past its oldest point there is no time.
        let finite = [LogLikeFormat::new(2, 2), LogLikeFormat::new(2, 2)];
        assert_eq!(serial_ago(&finite, 1.0, 3.0), Some(6.0));
        assert_eq!(serial_ago(&finite, 1.0, 3.5), None);
    }

    #[test]
    fn the_species_point_of_a_time_is_the_inverse_of_the_games_time_of_a_point() {
        // Right now is -1; a point's own time is that point (whole numbers) at every scale, up to the last, unbounded one.
        assert!((species_point_of_minutes(0.0) + 1.0).abs() < 1e-9);
        for point in [0, 1, 5, 11, 12, 13, 23, 24, 25, 60, 100, 200, 300, 400] {
            let minutes = minutes_of_point(point) as f64;
            let back = species_point_of_minutes(minutes);
            assert!((back - f64::from(point)).abs() < 1e-9, "point {point}: {minutes} minutes came back as {back}");
        }
        // Halfway between two points of one scale is halfway between their places; a longer time is a later place.
        let (a, b) = (minutes_of_point(2) as f64, minutes_of_point(3) as f64);
        assert!((species_point_of_minutes((a + b) / 2.0) - 2.5).abs() < 1e-9);
        let mut last = f64::MIN;
        for minutes in (0..200_000).step_by(997) {
            let place = species_point_of_minutes(f64::from(minutes));
            assert!(place > last);
            last = place;
        }
    }
}
