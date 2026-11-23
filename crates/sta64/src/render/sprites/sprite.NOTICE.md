# Sprite shader

`sprite.wesl` is a copy of Bevy's forward PBR fragment shader with one added line
(ADR 0009, S3; the changes are listed in the file's header):

| Here | Bevy |
| --- | --- |
| `sprite.wesl` | `bevy_pbr` 0.20.1, `src/render/pbr.wesl`, lines 1 to 114 (<https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_pbr/src/render/pbr.wesl>) |

Bevy is dual-licensed under MIT OR Apache-2.0. This copy is used under the MIT licence,
whose text, as shipped in the `bevy_pbr` 0.20.1 crate, follows.

MIT License

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
