//! RUSTibites Improved: the player's own words for naming species.
//!
//! The game names a species by two words of one list (the supporters' Latin names, `latin_names.rs`): a genus, which a species' descendants inherit for a few
//! generations, and a fresh specific epithet. A [`NamePool`] is a list of the player's own words that takes the place of that list ([`NameMode::Mine`]) or
//! shares it ([`NameMode::Both`]). Naming draws exactly one random number either way, so the name stream of a run (`SpeciesRegistry::name_rng`) is used
//! the same with a pool as without one; and with NO pool every name is made exactly as the game makes it, which is what the pinned runs hold.
//!
//! Nothing of this is in a save (a species' name is its two words, as ever), nothing is hashed, and the pool belongs to the run, not to a world: the viewer
//! sets it with [`set_name_pool`] when the player's list changes, and every world, whatever thread it ticks on, names its species from it from then on.

use std::sync::{Arc, RwLock};

use crate::latin_names::LATIN_NAMES;

/// How the player's words are used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameMode {
    /// Only the player's words: every word of the list is as likely as any other.
    Mine,
    /// Half of the draws are the player's words (every one as likely as any other), half the game's own list drawn as the game draws it.
    Both,
}

/// The player's words and how they are used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamePool {
    words: Vec<String>,
    mode: NameMode,
}

impl NamePool {
    /// A pool of `words` (none of them empty or with a space in it), or `None` when there are no words: an empty pool would name nothing.
    #[must_use]
    pub fn new(words: Vec<String>, mode: NameMode) -> Option<Self> {
        let words: Vec<String> = words.into_iter().filter(|word| !word.is_empty() && !word.contains(char::is_whitespace)).collect();
        (!words.is_empty()).then_some(Self { words, mode })
    }

    /// How many words the player gave.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether there are no words (never true of a pool [`NamePool::new`] made).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// How many different words a draw can give: the player's, and in [`NameMode::Both`] the game's too (what the draw loops of the name generator count by).
    #[must_use]
    pub fn reach(&self) -> usize {
        match self.mode {
            NameMode::Mine => self.words.len(),
            NameMode::Both => self.words.len() + LATIN_NAMES.len(),
        }
    }

    /// The word the draw `u` (a number in `0..1`) gives; `weight` is the one the game draws its own words with (4 for a genus, 1.5 for an epithet).
    #[must_use]
    pub fn pick(&self, u: f32, weight: f32) -> String {
        match self.mode {
            NameMode::Mine => self.words[uniform_index(u, self.words.len())].clone(),
            NameMode::Both => {
                if u < 0.5 {
                    self.words[uniform_index(u * 2.0, self.words.len())].clone()
                } else {
                    game_word((u - 0.5) * 2.0, weight).to_owned()
                }
            }
        }
    }
}

/// `floor(len * u)`, kept inside the list.
fn uniform_index(u: f32, len: usize) -> usize {
    ((len as f32 * u).floor() as usize).min(len - 1)
}

/// The game's `WeightedRandomPatron` for the draw `u`: `1 - u^weight` picks a word of the game's list (the biggest pledges first, so the words at the END of
/// the list are the common ones).
#[must_use]
pub fn game_word(u: f32, weight: f32) -> &'static str {
    let sample = 1.0 - u.powf(weight);
    let len = LATIN_NAMES.len();
    LATIN_NAMES[((len as f32 * sample).floor() as usize).min(len - 1)]
}

/// How many words the game's own list has.
#[must_use]
pub fn game_word_count() -> usize {
    LATIN_NAMES.len()
}

/// A whole species name ("Genus epithet") as the generator would make it for the draws `genus_u` and `epithet_u`, from `pool` or, with none, from the
/// game's list (the viewer shows samples of what a list gives).
#[must_use]
pub fn sample_name(pool: Option<&NamePool>, genus_u: f32, epithet_u: f32) -> String {
    let draw = |u: f32, weight: f32| pool.map_or_else(|| game_word(u, weight).to_owned(), |pool| pool.pick(u, weight));
    let genus = draw(genus_u, 4.0).trim().to_lowercase();
    let mut letters = genus.chars();
    let genus = letters.next().map_or_else(String::new, |first| first.to_uppercase().collect::<String>() + letters.as_str());
    format!("{genus} {}", draw(epithet_u, 1.5).trim().to_lowercase())
}

static POOL: RwLock<Option<Arc<NamePool>>> = RwLock::new(None);

/// Sets (or, with `None`, takes away) the player's pool: every species named from now on, in any world, is named from it.
pub fn set_name_pool(pool: Option<NamePool>) {
    let mut slot = POOL.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = pool.map(Arc::new);
}

/// The player's pool, when there is one.
#[must_use]
pub fn name_pool() -> Option<Arc<NamePool>> {
    POOL.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn a_pool_needs_words_and_drops_the_ones_that_would_break_a_name() {
        assert!(NamePool::new(Vec::new(), NameMode::Mine).is_none());
        assert!(NamePool::new(words(&["", "two words", "tab\tbed"]), NameMode::Both).is_none());
        let pool = NamePool::new(words(&["fluffy", "", "bubbles", "a b"]), NameMode::Mine).expect("two good words");
        assert_eq!((pool.len(), pool.reach(), pool.is_empty()), (2, 2, false));
        let both = NamePool::new(words(&["fluffy"]), NameMode::Both).expect("one word");
        assert_eq!(both.reach(), 1 + game_word_count());
    }

    #[test]
    fn only_my_words_gives_each_word_the_same_share_whatever_the_weight() {
        let pool = NamePool::new(words(&["a", "b", "c", "d"]), NameMode::Mine).unwrap();
        for weight in [4.0, 1.5] {
            let mut seen = [0usize; 4];
            for step in 0..4000 {
                let u = (step as f32 + 0.5) / 4000.0;
                let word = pool.pick(u, weight);
                seen[(word.as_bytes()[0] - b'a') as usize] += 1;
            }
            assert_eq!(seen, [1000; 4], "weight {weight}");
        }
        // The ends of the range stay inside the list.
        assert_eq!(pool.pick(0.0, 4.0), "a");
        assert_eq!(pool.pick(0.999_999_9, 4.0), "d");
        assert_eq!(pool.pick(1.0, 4.0), "d");
    }

    #[test]
    fn both_lists_share_the_draws_half_and_half_and_the_games_half_is_drawn_as_the_game_draws_it() {
        let pool = NamePool::new(words(&["mine"]), NameMode::Both).unwrap();
        let mut mine = 0;
        for step in 0..1000 {
            let u = (step as f32 + 0.5) / 1000.0;
            let word = pool.pick(u, 1.5);
            if word == "mine" {
                mine += 1;
            } else {
                assert!(LATIN_NAMES.contains(&word.as_str()), "{word}");
                // The same word the game's own draw gives for the stretched number.
                assert_eq!(word, game_word((u - 0.5) * 2.0, 1.5));
            }
        }
        assert_eq!(mine, 500);
    }

    #[test]
    fn the_games_own_draw_favours_the_end_of_its_list() {
        let len = game_word_count();
        assert_eq!(game_word(0.0, 4.0), LATIN_NAMES[len - 1], "u = 0 gives the last word");
        assert_eq!(game_word(0.999_999, 4.0), LATIN_NAMES[0]);
        let late = (0..1000).filter(|step| LATIN_NAMES[len / 2..].contains(&game_word((*step as f32 + 0.5) / 1000.0, 4.0))).count();
        assert!(late > 750, "{late}");
    }

    #[test]
    fn a_sample_name_is_a_capitalised_genus_and_a_lower_case_epithet_from_the_pool() {
        let pool = NamePool::new(words(&["FLUFFY", "bubbles"]), NameMode::Mine).unwrap();
        assert_eq!(sample_name(Some(&pool), 0.1, 0.9), "Fluffy bubbles");
        assert_eq!(sample_name(Some(&pool), 0.9, 0.1), "Bubbles fluffy");
        let from_the_game = sample_name(None, 0.3, 0.6);
        assert_eq!(from_the_game.split(' ').count(), 2, "{from_the_game:?}: the game's trailing spaces are left out of a sample");
        assert!(from_the_game.chars().next().unwrap().is_uppercase());
    }
}
