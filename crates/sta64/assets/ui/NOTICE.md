# UI assets

`font-regular.bin` and `font-bold.bin` are signed-distance atlases baked by
`crates/sta_fontbake` (a port of Armada II Refit's `ui/fontbake`, MIT,
Copyright (c) 2026 Cedric Haase; see `crates/a2fmt/LICENSE`) from **Noto Sans
Condensed 2.015**, Medium and SemiBold, as Refit's theme does.

Noto Sans is Copyright 2022 The Noto Project Authors
(https://github.com/notofonts/latin-greek-cyrillic) and licensed under the SIL
Open Font License 1.1 (`OFL-NotoSans.txt`, copied from Refit's vendored copy).
The OFL allows redistribution, embedding and modification, including in a
binary form such as this atlas, provided the licence and copyright notice
travel with it and the Reserved Font Name rule is respected. The atlas holds
glyph outlines as distance fields, derived from the unmodified fonts, so it is
a modified version for OFL purposes: it is not sold alone and is not named
"Noto". The source TTFs are not stored here.

Source files (sha256), as in Refit's `platform/vendor/noto-sans-condensed-2.015/SOURCE.txt`:

    e367c9cff3acf134f7f12a6ae7ce876d6dbe3aaa2f70121bb1f9ab66457a7196  NotoSans-CondensedMedium.ttf
    55ebf2188d1b9ff6b09affe3783c7704f8bd627586bf0a097ea4f6a8842f8cce  NotoSans-CondensedSemiBold.ttf

Regenerate: see `crates/sta_fontbake/NOTICE.md`.
