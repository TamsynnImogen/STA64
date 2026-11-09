# Front-end menu map

`map.ini` is copied unchanged from Armada II Refit
(https://github.com/cedrichaase/armada2-refit) at commit
4177bd948cd31f2fbcf00ebc822021ed1a192172:

| Here | Refit |
| --- | --- |
| `map.ini` | `render3d/content/menu/map.ini` |

The scene that reads it (`crates/sta64/src/frontend/scene.rs`) follows Refit's
`render3d/core/src/menu/mod.rs` and the "The menu scene" section of
`render3d/README.md`: the view camera and its sway, the blend between views,
the staggered decloak of a group that has a `CloakView`, the departure's
timings, the ships' bob and the planet's turn. A copy at `Menu/map.ini` in the
game install takes precedence over this one, as in Refit.

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
