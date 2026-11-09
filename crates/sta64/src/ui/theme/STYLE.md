# LCARS theme (UI-03)

The spec is Refit's **`ui/STYLE.md`** (https://github.com/cedrichaase/armada2-refit,
commit 4177bd9). Its `Form::Lcars` theme is what `Theme::default()` carries: same
colour roles and ARGB values, same type scale, same 4-unit space grid, same
elbow geometry. Values copied from it are MIT (see `assets/ui/NOTICE.md`, D10).
Units are Refit's: pixels of a 1080-line screen (no `[UI] Scale` yet).

## Tokens (`Theme`, `tokens.rs`)

| Role | Value | Use |
|---|---|---|
| `panel` | `#000000` | panel body, clear colour |
| `panel_hi` | `#C5CEF1` | bottom elbow |
| `accent2` / `button` | `#9C9CDE` | lavender: top elbow, side bar, buttons |
| `button_hot` / `button_down` | `#C0C0FF` / `#EEEEFF` | hover / pressed |
| `button_on` / `accent` / `title` / `fill` | `#F4B84C` | gold: on, selection, title, fills |
| `button_off` | `#2E2E44` | disabled |
| `accent3` | `#7EA8D4` | sky cap of the title bar |
| `text` / `text_dim` / `text_off` / `text_on` | `#DCDCFF` / `#9A9AC8` / `#55556E` / `#000000` | text, secondary, disabled, on a pill |
| `track` / `focus` | `#20203A` / `#FFE2A8` | slider and bar track, focus ring |
| `row_hot` / `row_sel` / `row_alt` | `#349C9CDE` / `#58F4B84C` / `#10FFFFFF` | list rows |
| `good` / `warn` / `bad` | `#5FD36A` / `#F2C23B` / `#E5483B` | status |
| `display/title/heading/text/small/micro_size` | 44 / 26 / 22 / 20 / 17 / 14 | type scale |
| `gap` / `pad` / `row_h` / `button_h` / `list_row_h` | 8 / 12 / 32 / 40 / 26 | space |
| `corners` / `elbow_inner` | 30 / 12 | elbow outer and inner radius |
| `side_bar` / `top_bar` / `bottom_bar` / `bar_gap` | 18 / 12 (title: `title_size`+6) / 10 / 5 | frame bars |
| `track_h` | 6 | slider and progress track |
| `caps_titles` / `caps_buttons` | true / true | capitals |

## Widgets (`widgets.rs`)

LCARS form, per STYLE.md "Components": flat pills (`BorderRadius::MAX`) whose
colour carries the state (lavender, light lavender, near-white, gold, dark),
black bold capitals at the right; a toggle is lavender off, gold on (hover eases
the gold half way to `button_hot`); a 6-unit slider/progress track with a gold
fill; list rows with `row_hot` / `row_sel` and a gold half-pill at the left of
the selected one; a panel with a lavender top elbow, pale bottom elbow, side bar,
the title in gold capitals in the top bar's gap, a sky cap and a gold end bar
(no shadow, no edge). Pointer state is `WidgetState`; `paint_widgets` colours from
it in `GameSet::Ui`; clicks become `ButtonClicked`, `ToggleChanged`,
`SliderChanged`, `ListSelected`.

Not yet from STYLE.md: eased hover (~100 ms; ours is instant), the keyboard focus
ring, check-box pill switch with a knob, striped bar fills (`bar_stripes`),
tabs, tooltip, text field, scroll bar, the `[UI] Scale` multiplier, and the other
forms (Shell, Blade, Wing, ...).

## Fonts

Refit's face is **Noto Sans Condensed** (Medium regular, SemiBold bold), licensed
under the **SIL OFL 1.1**, which allows redistribution, so the baked atlases ship:
`crates/sta64/assets/ui/font-regular.bin`, `font-bold.bin`, with `OFL-NotoSans.txt`
and `NOTICE.md`. They come from `crates/sta_fontbake` (port of Refit's `ui/fontbake`;
`cargo run --release -p sta_fontbake -- <noto dir> crates/sta64/assets/ui`).
`font.rs` loads them (`SdfFont::regular()/bold()`: glyph lookup, kerning, `measure`,
`layout` into pixel quads, `image()` as an R8 texture) and is tested.

**Follow-up (not done):** drawing the quads. The widgets still use Bevy's text because
an SDF UI material (a `UiMaterial` with a smoothstep-around-0.5 fragment shader and one
quad batch per label) is a separate change. The icon atlas (`fontbake/src/icons.rs`,
630 lines of SDF primitives) is not ported either.

## Gallery

`sta64 --ui-gallery` opens a window with one of every widget. Frame baseline
(local only, needs a GPU): take it manually, or with `sta-bench run --tier 2
--frames`. TOOL-04 frame capture (#94) is scene-driven (`--scene ... --headless --frames`) and has no gallery path yet; adding one is a follow-up.
