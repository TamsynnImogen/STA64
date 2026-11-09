# Fog shader

`fog.wesl` is ported from Armada II Refit (https://github.com/cedrichaase/armada2-refit)
at commit 4177bd948cd31f2fbcf00ebc822021ed1a192172:

| Here | Refit |
| --- | --- |
| `fog.wesl` | `render3d/mods/overlays/shaders/overlays.hlsl` (`ps_fog_bspline`, `finish`) |

The look's values in `fog.rs` (the fog's opacity 0.7, the edges' threshold 0.5 and
softness 0.2 (fog) and 0.1 (shroud), the rim's strength 1 and width 0.25) are the
defaults of `[Overlays]` in `render3d/mods/overlays/src/fog.rs`. The layers' colours are
not Refit's (its art is the engine's own textures, not in this repository).

MIT License

Copyright (c) 2026 Cedric Haase

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
