# Footage event log

A file the game writes about itself while it is played, so that someone cutting a screen
recording can tell, by wall clock, which game was running, when a cutscene or a fight was on,
which ships were in frame, and where the beats were (a kill, the end of the match, a pause). It
is a read-only observer: it changes nothing in the sim, the input or the picture, and it does not
touch determinism. Off unless asked for; off, none of it runs.

## Switching it on

- `--footage-log [<path>]` on the command line, or `STA64_FOOTAGE_LOG=<path>` in the environment
  (the flag wins). `0`, `off` and an empty value mean off.
- `<path>` is a file, a folder (an existing one, or a value ending in `/`: the file inside is
  named by its start), or nothing / `1` for the default place.
- Default place: `footage/<UTC stamp>.jsonl` in the per-user data directory, next to the
  `captures/` folder (`$XDG_DATA_HOME/sta64/footage/`, `~/.local/share/sta64/footage/`,
  `%LOCALAPPDATA%\sta64\footage\`, `~/Library/Application Support/sta64/footage/`; it follows
  `STA64_CAPTURE_DIR`, see `docs/TESTBENCH.md`). Never inside an install.
- The game logs one `info!` line with the path. A file that cannot be opened is a warning and the
  game carries on without the log.

Windowed runs only (the front end, `--map`, `--skirmish`, `--view`); the headless bench does not
write it.

## Format

JSONL, format tag `sta64-footage/1`: one compact object per line, flushed as it is written, so a
crash or a kill leaves a usable file. Every record has:

| Field | |
|---|---|
| `ms` | Unix epoch milliseconds. Read once at the start, then the monotonic time since is added: the stamps never jump backwards, but they do not follow the system clock after the start |
| `t` | seconds since the log started, 3 decimals |
| `tick` | the sim tick (`AnimClock`) while a match is on screen, else `null` |
| `type` | the record's name |

The records, with their fields besides those four (ids are sim entity ids; `owner` and `player`
are player numbers; numbers are rounded to 3 decimals; a field that is not known is `null`):

| `type` | When | Fields |
|---|---|---|
| `start` | the first frame | `format`, `build` (`version`, `commit`, `os`, `arch`, `debug`), `pid`, `started_at` (RFC 3339 UTC), `window` (`width`, `height`, `physical_width`, `physical_height`, or `null`) |
| `app_state` | every change of the app's state | `state` (`boot`, `front_end`, `loading`, `in_game`, `viewer`), `from` |
| `match` | once when a match becomes `in_game` | `profile` (the content profile, e.g. `Fo4`), `mod` (the active mod's folder), `mod_name` (its `[Mod] Name`, when the front end lists it), `mounted` (layer names, top first), `game`, `ui_profile`, `map`, `map_label`, `mode` (`mission`, `skirmish`, `map`, `net`), `campaign`, `mission` (index; missions only), `local_race`, `local_seat`, `players` (`id`, `team`, `race`, `control`) |
| `movie_start` | a pre-rendered movie is asked for | `id`, `name`, `target` (`fullscreen`, `panel`), `skippable` |
| `movie_end` | it is over | `id`, `name`, `outcome` (`completed`, `skipped`, `stopped`, `failed`), `reason` (failures) |
| `cutscene` | a script's cineractive turns on or off (a skip by the player counts) | `on`, `skippable` |
| `shot` | a camera shot of a cineractive | `shot` (number in this match), `from`, `at` (`{"entity":id}`, `{"pos":[x,y,z]}` or `{"path":name,"index":n}`), `seconds` |
| `title` | a script shows a title | `text`, `subtitle` |
| `mission_begin` / `mission_end` | the campaign flow starts / closes a mission | `map` (begin), `campaign`, `mission`; `outcome` (`won`, `lost`) and `outcome_tick` (end) |
| `verdict` | the match is decided for the local player | `result` (`victory`, `defeat`) |
| `eliminated` | a player is eliminated | `player` |
| `pause` | the game is paused or resumed | `on` |
| `destroyed` | a unit is destroyed | `id`, `by`, `class`, `owner` |
| `shields_down` | a unit's shields fail on a facing | `id`, `facing` (0 fore, 1 aft, 2 port, 3 starboard), `class`, `owner` |
| `built` | a unit is finished | `id`, `builder`, `class`, `owner` |
| `cloak` | a unit cloaks or decloaks | `id`, `on`, `class`, `owner` |
| `fight` | a fight starts or stops anywhere the local player can see | `on`, `units` (fighting now) |
| `fight_in_frame` | a fighting unit enters or leaves the main camera's frame | `on`, `units`, and on `on` the `classes` of those in frame |
| `cast` | the list of units in frame (see below) | `why` (`cutscene`, `shot`, `fight`), `shot`, `total`, `units` (`id`, `class`, `name`, `owner`, `race`, `x`, `y`, `size`) |
| `enter` / `leave` | during a cineractive, a unit enters or leaves the frame between casts | as a `cast` unit (`enter`); `id`, `class` (`leave`) |
| `camera` | the main camera's orbit pose moved (looked at about once a second) | `focus` (x, y, z), `yaw`, `pitch`, `dist` |
| `stop` | the app exits | `code` |

`class` is the unit's ODF name; `name` its on-screen name. A fight is the action camera's rule:
a unit is fighting for 6 seconds after it fired or was hit, and only units the local player sees
count. Single shots are not records.

### The cast

A unit is in frame when it is drawn (what the sensors show the local player; during a cineractive
everything but cloaked units, as the renderer does), its centre is in front of the main camera,
and its centre grown by its projected selection radius overlaps the viewport. `x`, `y` are the
centre in 0..1 of the viewport (top left is 0, 0; a big unit whose centre is off screen can read
slightly outside), `size` the projected diameter over the viewport height. A list holds the 40
largest, biggest first; `total` counts all.

- During a cineractive: a full `cast` about 0.3 s after the cineractive starts and after each
  `shot` (the scripted camera needs that long to get there; `why` says which), then `enter` and
  `leave` records between casts. A unit leaves only after 0.5 s out of frame.
- Outside one: no continuous list (it would flood the file). One `cast` when `fight_in_frame`
  turns on. The frame is looked at about 4 to 5 times a second.

## Lining it up with a recording

The recorder's sidecar holds the wall-clock start of each video (`started_at`). The offset of a
record in the video is `(ms - started_at_ms) / 1000` seconds, so a clip from a cutscene is the
span between a `cutscene` `on` and `off` record, a fight is `fight` on to off, and so on. Both
stamps are wall clock, so the machine's clock should be right and the recorder and the game on
the same machine; the log's `start` record gives `started_at` for a check against the first
frame. `t` is the same time from the log's own start, for reading without a recording.

## Limits

- Occlusion is not considered: a ship behind a planet or another ship is "in frame".
- Pre-rendered movies are known by file name only (`movie_start` / `movie_end`); their content is
  not logged. An in-engine cineractive has no name beyond the mission and the map; its shots and
  casts tell what was on screen.
- `game` is a cutting hint, not a rules decision: `a1` (Armada I), `a2` (Armada II), `dw` (Fleet
  Operations 3.x), `fo4`, `roots` (the Fleet Operations 4 profile with a mounted mod whose folder,
  name or parent layer contains "roots"), `native`. The raw `profile` and `mod` are logged next
  to it; trust those over the label.
- The `cast` waits for the camera: a shot followed by another within 0.3 s gets no cast of its own.
- A match played over the network has no `mounted` list and `mode` is `net`.
