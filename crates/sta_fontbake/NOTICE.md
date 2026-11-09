# sta_fontbake

`src/lib.rs` is a port of Armada II Refit's `ui/fontbake/src/lib.rs`
(https://github.com/cedrichaase/armada2-refit, commit
4177bd948cd31f2fbcf00ebc822021ed1a192172): the icon baker is not ported, the
font entry point takes a directory instead of Refit's vendor path.

MIT License, Copyright (c) 2026 Cedric Haase. See `../a2fmt/LICENSE` for the
licence text, which applies to this crate as well.

Usage (regenerates `crates/sta64/assets/ui/font-*.bin`; the output is
deterministic):

    cargo run --release -p sta_fontbake -- /usr/share/fonts/noto crates/sta64/assets/ui

The directory holds `NotoSans-CondensedMedium.ttf` and
`NotoSans-CondensedSemiBold.ttf`, version 2.015 (Arch: `noto-fonts`). The fonts
themselves are not stored in the repository; see `crates/sta64/assets/ui/NOTICE.md`.
