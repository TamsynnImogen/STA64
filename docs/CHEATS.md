# Single-player cheats

During a running match, press **Enter**, type the code exactly, then press **Enter** again.
**Escape** cancels. Codes are case-sensitive; spaces are significant. Typing blocks selection,
orders, camera movement, menu shortcuts and reveal shortcuts for that frame. The input is
available from a campaign, Instant Action, or a directly opened map. It is not a multiplayer
chat implementation: every cheat is refused in a network match, including diagnostic commands.

The mounted content's engine profile determines the codes; the faction and mod name do not.
The Native profile also offers the Fleet Operations sandbox commands.

| Code | Game | STA64 behavior |
|---|---|---|
| `showmethemoney` | Armada I / II | Add 2,000 Dilithium to the local player. |
| `showmethemoney` | Fleet Operations | Add 10,000 Dilithium, 10,000 of stock slot 1 (normally Tritanium), and 200 of slot 3 (normally Supply). Resource renaming does not change the slot. |
| `kobayashimaru` | Armada I / II; also available to FO missions | Force a running campaign mission to win immediately. Refused without a mission or after its outcome is final. |
| `kobayashimaru_lost` | Armada II; also available to FO missions | Force a running campaign mission to lose immediately, including a mission with unsupported script calls. Armada I does not recognize this code. |
| `expl` | Fleet Operations | Destroy selected live ships and stations. |
| `elim` | Fleet Operations | Remove all objects of the selected player's seat, leaving allied seats alone; the normal victory check follows. |
| `m` | Fleet Operations | Order selected mobile craft 200 world units forward, including an inspected enemy; ordinary pathing still applies. |
| `dis` | Fleet Operations | Drop selected ships' and stations' shields and keep the generator switched off until a script enables it again or the match ends. Maximum shields are preserved. |
| `version` | Fleet Operations | Show STA64's own version and its build commit when supplied. |
| `cine` | Fleet Operations | Toggle STA64's cinematic presentation through its existing cineractive event. It uses the existing camera/HUD and cineractive-end rules. |
| `fps` | Fleet Operations | Toggle a frame-rate counter. This diagnostic setting persists across matches and menus. |
| `dump` | Fleet Operations | Write the real simulation snapshot under the capture directory's `dumps/` folder. Each filename is new. The default on Linux is `~/.local/share/sta64/captures/dumps/`; `STA64_CAPTURE_DIR` overrides the capture root. |
| `crash` | Fleet Operations | Exit STA64 with status 1. It requests an application exit rather than causing a memory fault. |

`canofwhoopass` (Armada I tactical-AI mode) and `levelup` (Fleet Operations vessel promotion)
are recognized and explain their missing engine prerequisite. They do not report success or
queue a substitute effect. STA64 has neither the original tactical-process modes nor vessel
rank/replacement progression yet. These two cheats remain unimplemented.

Original mission chat-opening restrictions such as `DisableChat`, an Enter-waiting script step,
and the objectives display are not modeled by this input. It is a single-player testing input;
campaign mission endings retain LUA-29's exact string and already-ended checks.

The shield switch now has a real effect instead of only recording its state: shields drop,
stay down, and resume their authored recharge when enabled. Existing saved off-state shields
are zeroed before the next combat tick. The optional A2FOCheats module's 10,000-second automatic
shield recovery and its configurable five-resource money override are not implemented here.
`dis` uses the persistent boolean mission switch. The `elim` behavior follows the guide and
the repaired handler, rather than reproducing FO 3.2.7's accidental explosion-handler alias.

The undocumented FO debug registrations `cine`, `fps` and `dump` are exposed for local testing.
STA64's dump is a native STA64 snapshot; it is not an original-game memory dump. Commands commonly
reposted as Armada cheats, such as `avoidance`, `phonehome`, `nomoreships` and `youstopmecold`,
are not added without executable evidence that they work.

## Reveal controls (PR #837, UI-46)

By default, **Alt+F** lifts/restores fog and **Alt+V** lifts/restores shroud. At launch use `--reveal fog`,
`--reveal shroud` or `--reveal all`. Lift both to see the whole map. They remain client-side
presentation changes and are refused in network games. Clicking an enemy exposed by the cheat
may issue an ordinary attack order; that explicit order is recorded and replayed normally.

The existing `settings.cfg` file can rebind the reveal modifier and both action keys:

```text
reveal_modifier = AltLeft, AltRight
reveal_fog = KeyF
reveal_shroud = KeyV
```

Keys separated by commas are alternatives. Hold a bound modifier and press a bound action key;
for example, `reveal_modifier = ControlLeft, ControlRight` and `reveal_fog = KeyG` use Ctrl+G.
Older settings files keep the defaults. This uses the existing settings file and binding API;
there is no graphical binding editor yet. Menu and cheat entry still capture these keys, and
network games still refuse reveal changes.

## Recording and verification

Money, destruction, movement, shield switches and cinematic events go through the command
router under the existing reserved sandbox identity. The replay records the applied requests;
snapshots retain their normal simulation effects. No player gains edit privileges and the
network refusal runs before any edit or application exit. Forced mission endings use the
existing host-only campaign outcome mechanism; they are not replayed as simulation commands.
STA64 currently has no interactive replay player to host this input.

The resource-grant operation adds at simulation command application time, so repeated codes
and a purchase in the same tick cannot replace each other's stock. Its new appended TeamOp
variant requires wire protocol 15 (14 introduced mission switches); no snapshot
layout changes. The earlier TeamOp tags remain unchanged.

Rule evidence: cheat dispatch and resource constants read statically from the original engines;
Fleet Operations debug behavior also described by the [official single-player cheat guide](https://guide.fleetops.net/guide/basics/singleplayer-cheats).
The restored `m` movement distance comes from the local A2FOCheats implementation. Native timing
and graphical parity have not been measured. No original game content is included.
