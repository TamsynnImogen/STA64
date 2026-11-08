# ODF fixtures

Hand-written ODFs (no game data) for `sta_odf` tests. Each folder is one VFS
layer; tests list them top first (`mod`, `parent`, `base`).

- `fcruiser.odf` in all three layers: each includes its own name, so the mod's
  copy inherits the parent's, which inherits the base's (FO self-include).
- `fdestroy.odf` reaches its next version through `fdestroy_tweak.odf`.
- `cycle/`: `loop_a` → `loop_b` → `loop_c` → `loop_b`, a true cycle.
