# LCARS theme (UI-03)

Refit's `ui/STYLE.md` and `ui/fontbake` are not in this repo, so this is an
LCARS style written from the plan. When Refit's file is available, compare the
tokens below against it; any Refit-derived table or code must carry Refit's MIT
notice (D10).

## Tokens (`Theme` resource, `tokens.rs`)

| Token | Value | Use |
|---|---|---|
| `background` | `#000000` | clear colour, list rows |
| `orange` | `#FF9900` | primary frames, buttons |
| `peach` | `#FFCC99` | toggles, text |
| `lilac` | `#CC99CC` | secondary buttons |
| `sky` | `#99CCFF` | progress, tertiary buttons |
| `tan` / `red` | `#FFAA90` / `#CC6666` | warnings, alerts |
| `text` / `text_dark` | `#FFCC99` / `#000000` | text on black / on colour |
| `disabled` | `#555566` | disabled widgets |
| `hover_boost` / `press_boost` / `off_dim` | 1.15 / 0.75 / 0.45 | state brightness multipliers |
| `bar_height` | 28 px | pill and bar height |
| `rail_width` | 72 px | panel left rail |
| `elbow_radius` | 36 px | panel elbow corners |
| `gap` / `pad` | 6 / 10 px | spacing |
| `font_size` / `title_size` | 16 / 20 px | text |

## Widgets (`widgets.rs`)

`label`, `button`, `disabled_button`, `toggle`, `slider`, `progress_bar`,
`list`, `panel` (elbow frame with title bar). Pointer state is
`WidgetState` (Normal, Hovered, Pressed, Disabled), set by picking observers;
`paint_widgets` colours from it in `GameSet::Ui`. Clicks become messages
(`ButtonClicked`, `ToggleChanged`, `SliderChanged`, `ListSelected`).

## Fonts

Bevy's default font is used. No SDF/MSDF pipeline ships: there is no licensed
font in the repo and the `fontbake` port needs one. Follow-up: bake an open
licence font (e.g. an OFL LCARS-style face, with its licence file) into
`crates/sta64/assets/ui/` and set `TextFont::font` in the builders.

## Gallery

`sta64 --ui-gallery` opens a window with one of every widget. Frame baseline
(local only, needs a GPU): take it manually, or with `sta-bench run --tier 2
--frames`. TOOL-04 frame capture (#94) is scene-driven (`--scene ... --headless --frames`) and has no gallery path yet; adding one is a follow-up.
