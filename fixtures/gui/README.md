# GUI CFG fixtures

Hand-written GUI layouts (no game data) for `sta_content::gui_cfg` tests.

- `a2/`: an Armada II-style race file (`gui_bor.cfg`) whose own lines must
  beat the `gui_interface.cfg` it includes at its top.
- `fo/`: a Fleet Operations-style widescreen file (`gui_federation16x9.cfg`)
  that includes the aspect file before the race chain; the aspect file's
  values must win. `gui_interface.cfg` is reached twice and read once.
