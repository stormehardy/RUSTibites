//! The process-wide machine profile (see `bibites_core::calibration`). It is a test of its own, in its own process: the profile is global, and
//! the unit tests of the crate make worlds that would start from it.

use bibites_core::sim::{ParallelGates, MachineProfile, machine_profile, set_machine_profile};
use bibites_core::world::World;

#[test]
fn a_world_made_after_a_profile_is_set_starts_from_it_and_one_that_exists_takes_it_when_told() {
    assert_eq!(machine_profile(), None, "nothing set at the start");
    let before = bibites_core::calibration::synthetic_world(20, 1).expect("a world");
    assert_eq!(before.parallel_gates(), ParallelGates::SCALED);

    let profile = MachineProfile { workers: 3, pool_bibites: 640, pool_bodies: 1_600, start_level: 2 };
    set_machine_profile(Some(profile));
    assert_eq!(machine_profile(), Some(profile));

    // A world made now: the profile's first guess, and `set_threads(0)` gives the profile's workers.
    let mut after: World = bibites_core::calibration::synthetic_world(20, 1).expect("a world");
    let gates = after.parallel_gates();
    assert_eq!((gates.pool_bibites, gates.pool_bodies, gates.start_level), (640, 1_600, 2));
    assert!(gates.adaptive);
    after.set_threads(0);
    assert_eq!(after.threads(), 3, "the workers of the calibration, not every thread of the machine");
    after.set_threads(5);
    assert_eq!(after.threads(), 5, "a number given by hand wins");

    // A world that exists is told: automatic threads and the new first guess; one whose gates were set by hand keeps them.
    let mut old = before;
    old.set_threads(2);
    old.apply_machine_profile(true);
    assert_eq!(old.threads(), 3);
    assert_eq!(old.parallel_gates().pool_bibites, 640);
    let mut by_hand = bibites_core::calibration::synthetic_world(20, 1).expect("a world");
    by_hand.set_parallel_gates(ParallelGates::FIXED);
    by_hand.set_threads(2);
    by_hand.apply_machine_profile(false);
    assert_eq!(by_hand.threads(), 2, "not automatic: kept");
    assert_eq!(by_hand.parallel_gates(), ParallelGates::FIXED, "gates that do not measure were set by hand: kept");

    // The same ticks, whatever the profile: the profile is about time.
    let mut plain = bibites_core::calibration::synthetic_world(60, 9).expect("a world");
    plain.set_threads(1);
    plain.run(25);
    let mut profiled = bibites_core::calibration::synthetic_world(60, 9).expect("a world");
    profiled.set_threads(0);
    profiled.run(25);
    assert_eq!(bibites_core::sim::state_fingerprint(&plain), bibites_core::sim::state_fingerprint(&profiled));

    set_machine_profile(None);
    assert_eq!(machine_profile(), None);
}
