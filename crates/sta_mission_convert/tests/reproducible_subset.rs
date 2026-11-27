//! The converter's Lua preludes stay within what a mission state runs reproducibly (#1300):
//! no weak tables, no finalizers, no collector calls (the sandbox refuses or removes them).
//! The emitted mission bodies are not checked here yet (see TODO.md).

use sta_mission_convert::prelude::{A1_PRELUDE, ENGINE, PRELUDE};

#[test]
fn preludes_use_no_weak_tables_finalizers_or_collector_calls() {
    for (name, src) in [("PRELUDE", PRELUDE), ("ENGINE", ENGINE), ("A1_PRELUDE", A1_PRELUDE)] {
        for banned in ["__mode", "__gc", "collectgarbage"] {
            assert!(!src.contains(banned), "{name} uses {banned}");
        }
    }
}
