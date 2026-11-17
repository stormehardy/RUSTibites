# 36k Default per-bibite contact/occupancy extension

Tracked schema: `sample_schema.json`. Raw samples (gitignored): 
`testdata/parity/default/ecology_36k/contact_ext/samples_fast.jsonl` (periodic through 36k, ~100-tick cadence)
and `samples_early.jsonl` (ticks 7-600, ~5-tick cadence). Each row has per-live-bibite x/y/energy/health,
stomach/mouth state, mouth/plant contact tick counters and first-contact ticks, bite counters, and the nearest
plant pellet from the full pellet map (distance + coordinates/material). Occupancy summary uses the single
World zone radius 2000 from settings.bb8settings (radius relative 1.0); world settings are voidAvoidance False,
voidAvoidanceDistance 100, shadeAvoidance False, shadeOutsideOfBounds True, worldWrapping True.
