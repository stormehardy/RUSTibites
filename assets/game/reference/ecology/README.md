# Default 36,000-tick ecology capture (game)

Hidden Default newgame seed 12345, requested speed 8. The bridge run stopped on simTime=1200 s and
overshot to 47,993 ticks (effective ~40 ticks/s, achieved ~3.46x); summary/analysis uses the first
36,000 ticks (601 sampler rows, birth/death events, /d/feeding polls at exact ticks). Raw full-run
rows/events/feeding are under `testdata/parity/default/ecology_36k/` (gitignored); tracked summary:
`default_36k_summary.json`. Use for Rust long-run population/energy/plant/egg/death/feeding envelope
and speciation comparison.

**Egg/combat audit additions (2026-11-01):** `default_36k_combat.json` holds a compact per-attack
table reconstructed from the cumulative feeding polls (tick, attacker id, bite/damage/murder
deltas, cumulative totals, attacker energy/health/size) plus the two `killed` victim records
(energy/health/max health/damage/age). `default_36k_combat_events.csv` is the same attack rows in
CSV. Caveat: non-fatal bites do not expose a victim id; "damage-only" poll rows are sustained bite
damage, while first-bite rows have `d_bites>0`. Egg counts need fine (~60-tick) sampling: the
601-row series peaks at 6 eggs; a coarse 1,000-tick Rust sample falsely suggested 0–1, while fine
sampling shows Rust peaks at 8. Do not compare egg counts from coarse samples.

`default_36k_rust_contacts.json` is the matching Rust 36k mouth/attack trace after the
sustained-bite body fix (`f76acf6`, Default seed12345): 35,463 contact samples, 8 attacker-victim
pairs, 12 consecutive-tick intervals (median1,960/max9,174 ticks), brain outputs, relative
distance/bearing, attacker bite force and victim armor/radii/max-health, plus per-attacker
first-bite/damage totals. `default_36k_rust_intervals.csv` has the compact interval rows. In this
trace attack output is 0.5 and eat 0.842 for every contact, targets include bibites (not
pellet-only), and all-record attack damage is **908.6 from 12 first bites** (one kill; game is
632/21/3). The earlier pre-fix trace (931 contacts/51.7 damage) is superseded.
`default_36k_combat.json` also includes game contact intervals reconstructed from feeding-poll
cumulative counters (poll resolution, not per tick).

`default_36k_recruitment.json` compares first-36k recruitment/starvation: game first-36k sampler
rows are **35 births/17 deaths (16 starved,1 killed), avg pop15.5, births/pop2.26, deaths/pop1.10,
starved age mean148.9 s/maxEnergy83.3, eggs max6**; Rust seed12345 is 31/25 (24 starved,1 killed),
avg8.55, births/pop3.62, deaths/pop2.92, starved age298 s/maxE107.7, eggs9; seed67890 is35/21
(21 starved,0 killed), avg11.79, 2.97/1.78, age171.1 s/maxE75.3, eggs9. The game events file is
full-run (39/25,2 killed) and must not be mixed with first-36k rows; first-36k kills are 1 vs1.
Rust per-capita births/eggs are higher, so the residual is per-capita starvation/survival, not
recruitment or combat.

`default_36k_death_budget.json` has per-starved-death histories (age/maxE, last energy/stomach,
last-meal time, last-20s energy slope/contact rates) for game and both Rust seeds.
`default_36k_death_cohorts.json` adds the dead-vs-live 60-s comparison: starved dead have
plant-contact rate 0.0–0.94/s vs live 9.1–15.6/s, stomach-positive fraction 0.06–0.12 vs
0.64–0.76, and empty stomachs/energy≈0 at death; game last meal≈135 s, Rust≈155–209 s. Fat is not
exposed by either feeding poll.

`default_36k_occupancy.json` now includes the steering/occupancy trace: Rust off-zone
records have zero plant senses, Accelerate0.415 vs0.240 in-zone, Rotate/Herding0, speed11.23
vs4.66, radial velocity+5.8/s; 65% of low-energy records are off-zone and faster. Game first-600
positions are all inside r<2000 (r_p90 1645.8; per-tick radial drift+0.805/s, only 45.5% positive, speed3.29 vs Rust7.56), so a later game position
capture is needed before adding any zone/void steering. See `behavior_steering` and
`game_early` keys.




`default_36k_occupancy_later.json` compares later game positions (600–27,000 from
`samples_fast.jsonl`) with Rust: off-zone 19.6% game vs24.0/21.0 Rust, similar radial drift
(+2.59 vs+2.49/+2.43/s), exits11 vs13/8, returns1 vs0. The divergence is **local in-zone nearest
plant**: game p50 6.35 vs Rust25.18/9.43; all-record p50 10.95 vs92.93/22.26, while off-zone
distances are similar. Occupancy policy and body steering are not the differentiator; world pellet
spatial distribution/local accessibility is. Speeds are position-delta means (game cadence
~70–100 ticks, Rust30), not native vx/vy.

`default_36k_pellets_compare.json`: game/Rust pellet radial distributions match (early p50 833/910;
late 879/882, 10 vs0 outside r2000), CentricGradual formulas match, and collider radii agree after
the 2.5x prefab factor. The remaining difference is within-zone bibite occupancy: in-zone radius
p50 672 game vs1381/857 Rust12345/67890, nearest pellet 6.5 vs25.2/9.4. This is foraging/occupancy,
not pellet distribution.

`default_36k_ensemble.json`: six Rust seeds (1,2,3,4,12345,67890) vs the game later positions.
Game occupancy is inside the Rust seed spread — in-zone radius p50 672 vs Rust
678/888/1381 (min/median/max), radius p90 1530 vs1470/1591/1753, nearest p50 6.48 vs
5.33/8.16/25.18, nearest p90 102 vs97.7/107.5/212, births35 vs28/32/35, deaths17 vs17/23/25.
Seed12345 is the outlier; occupancy is lineage/RNG variance, not systematic. Plant-contact rate
remains outside: game18.04/s vs Rust8.75/12.89/14.32, and plant_n650 vs592/617/628 (separate
task-119 feeding gap).

`default_36k_contact_matched.json`: matched in-zone radius 600–27,000. Game plant-contact22.37/s vs Rust
15.14, but mouth targets match (0.573 vs0.575), plant targets0.509, stomach10.19 vs10.67, active
fraction0.618 vs0.528. Per-contact ingestion is higher in Rust (0.268 vs0.186 E/contact-tick, body
report); so the gross feeding gap is contact exposure/turnover + live population/lineage weighting, not
target acquisition. Contact gap remains body/task-119; C19 occupancy closes on ensemble metrics.

`default_36k_encounters.json`: contact counters normalize by **actual tick rate** (game39.99 vs Rust30).
Per-update contact: all0.451 vs0.417, in-zone0.559 vs0.505, nearest0-5u0.981 vs0.992, 5-10u0.751 vs0.670;
outer 1000-2000u Rust is higher. Rust plant-contact runs are long (159 runs, median433/max4034 ticks), target
counts/stomach match, so no bite knockback/cooldown/target-expiration interruption is implicated. The per-second
gap is mostly the 40-vs-30 TPS measurement plus live/lineage weighting; no body mouth patch indicated.
