# Ring focused FOV/seen-cache capture (post task-128)

Hidden Ring base, seed12345, 600 ticks at 1x, polled 0.05 s for per-bibite FOV seen-array counts/ids, needToSee/radius/angle, plant/meat/bibite range counts and stomach digestion fields. `ring_fov.json` has the initial/empty state, per-id change timeline/first nonempty tick and digestion first/last; raw under testdata/parity/fov/ring_fov. Native game stomach components expose per-tick amounts/rates, not cumulative digestion counters.
