# Persistent shield visibility

RND-32 adds the optional A2FOExtensions object ODF command
`alwaysShowShields = 1`. The default is off. The renderer reads the effective
ODF after ordinary includes and layer resolution, so an explicit zero in a
child disables its parent's setting.

A configured ship or station has one continuous shield outline while its
current shields are positive. At zero the outline disappears; recharge
restores it. Hit and collapse effects remain independent. The outline follows
the rendered unit and is suppressed while cloaked, outside the local player's
perception, or without a live render root. Removing a unit or leaving a match
cleans up its outline and private material.

This is native client presentation (D1, D2). It uses existing world views and
resolved ODF metadata, without adding rules, simulation state, content digest
fields or save/network formats (D9, D10). No original executable, hook or DLL
is loaded. The source rule is read from A2FOAlwaysShowShields' implementation
and documentation (static); this does not establish matching original-game
pixels.

The first implementation and verification details follow in this package's
PR. Manual comparison remains a separate acceptance gate.
