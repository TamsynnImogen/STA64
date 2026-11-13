# Captain and registry companions

UI-35 adds A2FOCraftIdentity's optional `possibleCaptainNames` and
`possibleCraftRegistry` ODF lists to the selected-object HUD. Each companion
uses the same original source row as the selected `possibleCraftNames` entry.
Empty entries keep their place, duplicate ship names remain distinct rows,
and missing or out-of-range companions are blank rather than wrapped.
Ordinary resolved ODF inheritance applies; an explicit empty child list clears
its inherited companions.

STA64's existing deterministic ship-name choice stays unchanged. The renderer
retains the original row even when blank craft-name entries are skipped by
that existing policy. A fallback to `unitName` or the ODF stem has no selected
name row and therefore no companions. No second random choice is made.
This does not import Armada's persisted native name index or add native
rename/save compatibility. STA64 entity identity and the same ODF continue to
produce the same cosmetic choice.

The medium selected panel reads `infoSingleCaptainTextArea` and
`infoSingleRegistryTextArea`; the tall producer panel reads
`infoBuildCaptainTextArea` and `infoBuildRegistryTextArea`. An omitted tall
property inherits its medium equivalent independently. If the modern medium
area is absent, the legacy `captainName` or `shipRegistry` rectangle is used.
Explicit zero or negative width/height hides a field and stops fallback.
Missing rectangles also hide it. Placement uses the existing scaled info-panel
origin, without changing stock layout or panel sizes.

Colours use the matching `infoBuild*TextColor` / `infoSingle*TextColor`
property, then `captainNameColor` or `shipRegistryColor`, then `infoTextColor`,
then the current theme's text colour. `shipNameColor` is not a companion
fallback. An independent tall colour does not require a tall rectangle.
Text uses the existing HUD font pipeline. Empty identities, no selection,
group selection and the low mouseover strip display no companion fields.

The rule is read from A2FOCraftIdentity's source documentation (static).
This is native cosmetic presentation under D1/D2 and D9/D10: no DLL loading,
gameplay RNG, simulation state, content digest, save or network format change.
No original game files are modified or bundled. Native visual acceptance is
separate from the synthetic model and headless HUD checks.
