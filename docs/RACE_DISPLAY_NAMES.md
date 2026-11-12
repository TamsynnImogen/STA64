# Faction display names

Faction ODF `displayKey` is presentation metadata, independent of `name` (the
canonical race ID used by unit definitions and game setup).

The race picker resolves the selected playable faction first, then renders:

1. `displayKey` through the active mount's `Dynamic_Localized_Strings.h`.
2. The literal `displayKey` if the table has no matching key.
3. The existing race identifier if `displayKey` is absent or blank.

For example, Classic's `GUI_RD_FEDERATION` resolves to `Federation`, while FO4's
literal `Federation` still displays correctly without a translation entry.
Keys and literal captions preserve authored case; normal ODF inheritance applies.
No faction captions or translation entries are hardcoded in the runtime.

The existing `StringTable` loader supplies native file decoding and mount-layer
resolution. The UI's `Strings` resource retains that table separately from its
STA64 call-site aliases. Same-language mod replacement and mount removal reload
it; changing the localization resource redraws setup captions. Existing `tr`
alias and STA64 language-file behavior remains unchanged.

Race eligibility, ordering, cycling and `PlayerSetup.race` continue to use the
canonical IDs. No simulation, save, protocol, dependency or original-data change
is involved. This implements D1/D7 from MODDING_DECISIONS.md; it adds no new
string overlay format.

Tests cover inherited/literal/missing/blank display keys, native table lookup and
fallback, UTF-8/Windows-1252 table decoding, same-language mount changes, actual
rendered setup labels, caption refresh and unchanged launched race IDs. The
optional `installed_race_display_keys_resolve_federation_caption` test uses
`STA64_TEST_INSTALL` and `STA64_TEST_MOD` to exercise an installed Classic/FO4
faction and its real string table without launching a window.
