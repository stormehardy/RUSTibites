//! RUSTibites Improved: the gene selector tower.
//!
//! The game's colour killer picks bibites by their colour: every `period` seconds it looks at the bibites within its `radius` and kills the first one whose hue is
//! OUTSIDE the tolerance around the target hue (the compiled polarity: `ColorKiller` in `tools.rs`), so only the target colour survives. The gene selector is the same
//! tower for a GENE: it targets one gene of the bibites, has a range of values (a `min` and a `max` in the gene's own units), and every cycle kills bibites whose value is
//! outside the range (selection, as the colour killer: the default) or inside it (a cull, `kill_inside`). The rest follows the colour killer's rules: one scan per
//! cycle over at most 128 bibites in id order, the first one found or the one furthest off target (`any_or_most`), the radius and the period; plus `kills_per_cycle`,
//! how many a cycle kills (1 from the start).
//!
//! The game has no such tower, so it is never written into a save (the game-format save writer leaves it out; the viewer keeps these towers in a file beside the
//! save) and it acts only while [`crate::improved::Feature::GeneSelectorTower`] is on. With none in a world nothing of this runs.

use crate::genome::{GENE_DEFS, N_GENES};
use crate::tools::TowerBase;

/// The default scan radius (the colour killer's).
pub const GENE_SELECTOR_RADIUS: f32 = 400.0;
/// The default seconds between cycles (the colour killer's).
pub const GENE_SELECTOR_PERIOD: f32 = 100.0;
/// The most bibites a cycle may kill.
pub const GENE_SELECTOR_MAX_KILLS: u32 = 50;
/// The gene a new tower targets: `Diet`, the one the player most often steers (index into [`GENE_DEFS`]).
pub const GENE_SELECTOR_DEFAULT_GENE: u8 = crate::genome::Gene::Diet as u8;

/// A gene selector tower.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneSelector {
    /// Common tower state (the position, the serialized id, the zone it follows: as the colour killer, it goes where its zone goes).
    pub base: TowerBase,
    /// The targeted gene: an index into [`GENE_DEFS`] (`Gene as u8`).
    pub gene: u8,
    /// The lowest value of the range, in the gene's own units.
    pub min: f32,
    /// The highest value of the range, in the gene's own units.
    pub max: f32,
    /// `false`: kill what is outside the range (selection, the colour killer's polarity); `true`: kill what is inside it.
    pub kill_inside: bool,
    /// `false`: kill the first candidate found; `true`: the one furthest off target (outside: the furthest out of the range; inside: the deepest in it).
    pub any_or_most: bool,
    /// How many bibites a cycle kills at most (1 to [`GENE_SELECTOR_MAX_KILLS`]).
    pub kills_per_cycle: u32,
    /// Scan radius.
    pub radius: f32,
    /// Seconds between cycles.
    pub period: f32,
    /// Accumulator toward the next cycle.
    pub progress: f32,
    /// Bibites killed so far (a runtime count).
    pub killed: u32,
}

impl GeneSelector {
    /// A tower at `pos` with the colour killer's radius and period, targeting [`GENE_SELECTOR_DEFAULT_GENE`] with the gene's WHOLE range: nothing is outside it, so a
    /// new tower kills nothing until the player narrows the range.
    #[must_use]
    pub fn new(save_id: u64, pos: crate::math::Vec2) -> Self {
        let mut tower = Self {
            base: TowerBase::new(save_id, pos),
            gene: GENE_SELECTOR_DEFAULT_GENE,
            min: 0.0,
            max: 0.0,
            kill_inside: false,
            any_or_most: false,
            kills_per_cycle: 1,
            radius: GENE_SELECTOR_RADIUS,
            period: GENE_SELECTOR_PERIOD,
            progress: 0.0,
            killed: 0,
        };
        tower.set_gene(GENE_SELECTOR_DEFAULT_GENE);
        tower
    }

    /// The definition of the targeted gene.
    #[must_use]
    pub fn def(&self) -> &'static crate::genome::GeneDef {
        &GENE_DEFS[usize::from(self.gene).min(N_GENES - 1)]
    }

    /// Targets another gene; the range becomes the gene's whole range.
    pub fn set_gene(&mut self, gene: u8) {
        self.gene = gene.min((N_GENES - 1) as u8);
        let def = self.def();
        self.min = def.min;
        self.max = def.max;
    }

    /// Sets the range (kept inside the gene's own range and in order: `min <= max`).
    pub fn set_range(&mut self, min: f32, max: f32) {
        let def = self.def();
        let (lo, hi) = (def.min, def.max);
        let (min, max) = if min <= max { (min, max) } else { (max, min) };
        self.min = min.clamp(lo, hi);
        self.max = max.clamp(lo, hi);
    }

    /// Sets the lowest value of the range; the highest follows when it would be below it.
    pub fn set_min(&mut self, min: f32) {
        let max = self.max.max(min);
        self.set_range(min, max);
    }

    /// Sets the highest value of the range; the lowest follows when it would be above it.
    pub fn set_max(&mut self, max: f32) {
        let min = self.min.min(max);
        self.set_range(min, max);
    }

    /// Sets how many a cycle kills.
    pub fn set_kills_per_cycle(&mut self, kills: u32) {
        self.kills_per_cycle = kills.clamp(1, GENE_SELECTOR_MAX_KILLS);
    }

    /// Advances the cycle timer; returns `true` when a scan is due (`ColorKiller::tick`).
    pub fn tick(&mut self, dt: f32) -> bool {
        self.progress += dt;
        if self.progress < self.period {
            return false;
        }
        self.progress -= self.period;
        true
    }

    /// Whether `value` is inside the range (the ends belong to it).
    #[must_use]
    pub fn contains(&self, value: f32) -> bool {
        value >= self.min && value <= self.max
    }

    /// Whether a bibite with this gene value is the tower's target to kill: outside the range, or with `kill_inside` inside it. A value that is not a number is
    /// never killed.
    #[must_use]
    pub fn wants_kill(&self, value: f32) -> bool {
        if value.is_nan() {
            return false;
        }
        self.contains(value) == self.kill_inside
    }

    /// How far off target a killable value is, for `any_or_most`: outside the range, the distance to its nearest end; inside it, the distance to its nearer end
    /// (the deepest bibite scores most).
    #[must_use]
    pub fn off_target(&self, value: f32) -> f32 {
        if self.contains(value) {
            (value - self.min).min(self.max - value)
        } else if value < self.min {
            self.min - value
        } else {
            value - self.max
        }
    }
}

/// What a cycle of `selector` kills among `candidates` `(id, value of the targeted gene)`, which come in id order: at most the first 128 are looked at (the colour
/// killer's cap), the killable ones are taken first-found or furthest-off-target (`any_or_most`, ties keep the id order), up to `kills_per_cycle`. The returned
/// positions index `candidates`.
#[must_use]
pub fn pick_kills(selector: &GeneSelector, candidates: &[f32]) -> Vec<usize> {
    let looked_at = candidates.len().min(crate::tools::COLOR_KILLER_MAX_TARGETS);
    let want = selector.kills_per_cycle.clamp(1, GENE_SELECTOR_MAX_KILLS) as usize;
    let killable = (0..looked_at).filter(|&at| selector.wants_kill(candidates[at]));
    if selector.any_or_most {
        let mut ranked: Vec<usize> = killable.collect();
        // Stable: equal scores keep the id order.
        ranked.sort_by(|&a, &b| selector.off_target(candidates[b]).total_cmp(&selector.off_target(candidates[a])));
        ranked.truncate(want);
        ranked
    } else {
        killable.take(want).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::Gene;
    use crate::math::Vec2;

    fn tower() -> GeneSelector {
        GeneSelector::new(7, Vec2::new(10.0, 20.0))
    }

    #[test]
    fn a_new_tower_targets_diet_over_its_whole_range_with_the_colour_killers_radius_and_period_and_kills_nothing() {
        let tower = tower();
        let def = &GENE_DEFS[Gene::Diet as usize];
        assert_eq!((tower.gene, tower.min, tower.max), (Gene::Diet as u8, def.min, def.max));
        assert_eq!((tower.radius, tower.period, tower.kills_per_cycle), (400.0, 100.0, 1));
        assert!(!tower.kill_inside && !tower.any_or_most);
        // The whole range is inside: selection (kill outside) finds nobody to kill.
        for step in 0..=10 {
            let value = def.min + (def.max - def.min) * step as f32 / 10.0;
            assert!(!tower.wants_kill(value), "{value}");
        }
        assert_eq!(pick_kills(&tower, &[def.min, def.max, 0.5 * (def.min + def.max)]), Vec::<usize>::new());
    }

    #[test]
    fn the_range_is_kept_in_order_and_inside_the_genes_own_range() {
        let mut tower = tower();
        let def = *tower.def();
        tower.set_range(0.6 * def.max, 0.2 * def.max);
        assert!(tower.min <= tower.max && (tower.min - 0.2 * def.max).abs() < 1e-6, "{} {}", tower.min, tower.max);
        tower.set_range(def.min - 5.0, def.max + 5.0);
        assert_eq!((tower.min, tower.max), (def.min, def.max));
        tower.set_range(0.3 * def.max, 0.5 * def.max);
        // A lowest value above the highest drags the highest along, and the other way round.
        tower.set_min(0.9 * def.max);
        assert_eq!((tower.min, tower.max), (0.9 * def.max, 0.9 * def.max));
        tower.set_max(0.1 * def.max);
        assert_eq!((tower.min, tower.max), (0.1 * def.max, 0.1 * def.max));
        // Another gene: the range is that gene's whole range.
        tower.set_gene(Gene::SizeRatio as u8);
        let size = GENE_DEFS[Gene::SizeRatio as usize];
        assert_eq!((tower.gene, tower.min, tower.max), (Gene::SizeRatio as u8, size.min, size.max));
        tower.set_gene(250);
        assert_eq!(usize::from(tower.gene), N_GENES - 1, "a gene out of the list is the last one");
        tower.set_kills_per_cycle(0);
        assert_eq!(tower.kills_per_cycle, 1);
        tower.set_kills_per_cycle(10_000);
        assert_eq!(tower.kills_per_cycle, GENE_SELECTOR_MAX_KILLS);
    }

    fn ranged(min: f32, max: f32) -> GeneSelector {
        let mut tower = tower();
        tower.min = min;
        tower.max = max;
        tower
    }

    #[test]
    fn selection_kills_what_is_outside_the_range_and_a_cull_what_is_inside_and_the_ends_belong_to_the_range() {
        let mut tower = ranged(0.2, 0.6);
        assert!(tower.wants_kill(0.1) && tower.wants_kill(0.7) && !tower.wants_kill(0.4));
        assert!(!tower.wants_kill(0.2) && !tower.wants_kill(0.6), "the ends are inside");
        tower.kill_inside = true;
        assert!(!tower.wants_kill(0.1) && !tower.wants_kill(0.7) && tower.wants_kill(0.4));
        assert!(tower.wants_kill(0.2) && tower.wants_kill(0.6));
        assert!(!tower.wants_kill(f32::NAN), "a value that is not a number is left alone");
    }

    #[test]
    fn a_cycle_takes_the_first_killable_bibites_in_id_order_or_the_furthest_off_target() {
        let values = [0.4, 0.9, 0.5, 0.05, 0.7, 0.25];
        // Selection of 0.2..0.6: the killable ones are at 1 (0.9), 3 (0.05) and 4 (0.7).
        let mut tower = ranged(0.2, 0.6);
        assert_eq!(pick_kills(&tower, &values), [1]);
        tower.kills_per_cycle = 2;
        assert_eq!(pick_kills(&tower, &values), [1, 3]);
        tower.kills_per_cycle = 10;
        assert_eq!(pick_kills(&tower, &values), [1, 3, 4]);
        // The furthest off target first: 0.9 (0.3 out), 0.05 (0.15 out), 0.7 (0.1 out).
        tower.any_or_most = true;
        tower.kills_per_cycle = 2;
        assert_eq!(pick_kills(&tower, &values), [1, 3]);
        tower.kills_per_cycle = 1;
        assert_eq!(pick_kills(&tower, &values), [1]);
        // A cull of the same range: inside are 0 (0.4, 0.2 in), 2 (0.5, 0.1 in) and 5 (0.25, 0.05 in): the deepest first.
        tower.kill_inside = true;
        tower.kills_per_cycle = 3;
        assert_eq!(pick_kills(&tower, &values), [0, 2, 5]);
        tower.any_or_most = false;
        assert_eq!(pick_kills(&tower, &values), [0, 2, 5], "first found: the id order");
        // Equal scores keep the id order (a range 0.25 to 0.75: 0.0 and 1.0 are both exactly 0.25 off).
        let mut tower = ranged(0.25, 0.75);
        tower.any_or_most = true;
        tower.kills_per_cycle = 4;
        assert_eq!(pick_kills(&tower, &[0.0, 1.0, 0.0, 1.0]), [0, 1, 2, 3]);
    }

    #[test]
    fn only_the_first_hundred_and_twenty_eight_bibites_are_looked_at() {
        let tower = ranged(0.4, 0.6);
        let mut values = vec![0.5; 200];
        values[130] = 0.0; // killable, but beyond the cap
        assert!(pick_kills(&tower, &values).is_empty());
        values[100] = 0.0;
        assert_eq!(pick_kills(&tower, &values), [100]);
    }

    #[test]
    fn the_cycle_timer_is_the_colour_killers() {
        let mut tower = tower();
        tower.period = 2.0;
        assert!(!tower.tick(1.0) && !tower.tick(0.5) && tower.tick(0.5));
        assert!((tower.progress - 0.0).abs() < 1e-6);
        assert!(tower.tick(2.0) && tower.progress == 0.0);
    }
}
