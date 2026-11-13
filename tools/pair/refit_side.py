#!/usr/bin/env python3
"""Original-game side of the TOOL-15 paired-scene comparison harness.

Runs one pair file (bench/pairs/*.ini) on Refit's test bench (a headless sway
display, never the user's screen): stock Armada II ("a2") or Fleet Operations:
Roots ("roots"). Writes marks, objects, frames and refit.json into --out, for
the Rust side to compare.

    python3 tools/pair/refit_side.py --pair bench/pairs/move.ini --game a2 --out DIR \
        [--refit CHECKOUT] [--dry-run] [--timeout 600]

All timing is in simulation seconds from the `t=` field of Scene.marks, relative
to the `ready` mark. Wall clock is only used for polling and the timeout.
Standard library only.
"""
import argparse
import fcntl
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import time

KNOWN_GAMES = ("a1", "a2", "roots")
SUPPORTED_GAMES = ("a2", "roots")
READY_TIMEOUT = 180.0
STALL_WALL = 45


class PairError(Exception):
    pass


# ---------------------------------------------------------------- INI parsing

def parse_ini(text):
    """[(section, [(key, value), ...]), ...] in file order, duplicates kept.

    Lines: `[section]`, `key=value`, `;`/`#` comments, blank. Keys before any
    section go under section ''. Not configparser: At= repeats.
    """
    sections = []
    cur = None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line[0] in ";#":
            continue
        if line.startswith("[") and line.endswith("]"):
            cur = (line[1:-1].strip(), [])
            sections.append(cur)
            continue
        if "=" not in line:
            continue
        k, v = line.split("=", 1)
        if cur is None:
            cur = ("", [])
            sections.append(cur)
        cur[1].append((k.strip(), v.strip()))
    return sections


def apply_game(sections, game):
    """`<game>.<Key>` replaces `<Key>` in its section; other games' keys are dropped."""
    out = []
    for name, items in sections:
        own = {}
        for k, v in items:
            pre, dot, rest = k.partition(".")
            # `At=` repeats and is never overridden per game (as in sta-bench).
            if dot and pre.lower() == game and rest.lower() != "at":
                own[rest.lower()] = (rest, v)
        new = []
        placed = set()
        for k, v in items:
            pre, dot, rest = k.partition(".")
            if dot:
                # Every prefixed key is some game's (a2, roots, fake for STA64's
                # fake content); Refit's own keys never contain a dot.
                continue
            if k.lower() in own:
                if k.lower() not in placed:
                    new.append(own[k.lower()])
                    placed.add(k.lower())
                continue
            new.append((k, v))
        for lk, kv in own.items():
            if lk not in placed:
                new.append(kv)
        out.append((name, new))
    return out


def get(items, key, default=None):
    """Last value of key (case-insensitive) in an item list."""
    r = default
    for k, v in items:
        if k.lower() == key.lower():
            r = v
    return r


def section_items(sections, name):
    r = []
    for n, items in sections:
        if n.lower() == name.lower():
            r += items
    return r


class Pair:
    pass


def secs(v, what):
    """Sim seconds, `3`, `3.2` or `3.2s` (as sta-bench reads them)."""
    v = v.strip()
    try:
        x = float(v[:-1] if v.endswith("s") else v)
    except ValueError:
        raise PairError("%s: %r is not seconds" % (what, v))
    if x < 0:
        raise PairError("%s: %r is negative" % (what, v))
    return x


def load_pair(text, game):
    """Parse a pair file for `game` into a Pair (raises PairError on refusal)."""
    if game == "a1":
        raise PairError("Armada I (a1) is not supported by the paired-scene harness; use a2 or roots")
    if game not in SUPPORTED_GAMES:
        raise PairError("unknown game %r (use a2 or roots)" % game)
    sections = parse_ini(text)
    pair_raw = section_items(sections, "Pair")
    games = [g.strip().lower() for g in (get(pair_raw, "Games", "") or "").split(",") if g.strip()]
    if game not in games:
        raise PairError("this pair does not list game %r (Games=%s)" % (game, ",".join(games) or "none"))
    resolved = apply_game(sections, game)
    p = Pair()
    p.game = game
    items = section_items(resolved, "Pair")
    p.games = games
    if get(items, "Duration") is None:
        raise PairError("[Pair] needs Duration=<sim seconds>")
    p.duration = secs(get(items, "Duration"), "Duration")
    p.frames = [secs(x, "Frames") for x in re.split(r"[,\s]+", get(items, "Frames", "").strip()) if x]
    m = re.fullmatch(r"(\d+)x(\d+)", get(items, "Size", "1280x720").strip().lower())
    if not m:
        raise PairError("bad Size= %r" % get(items, "Size"))
    p.size = (int(m.group(1)), int(m.group(2)))
    # `rclick` (a player's right-click): where the RTS camera centres an object
    # on screen; default the middle of the window.
    uc = get(items, "UiCenter")
    if uc:
        mc = re.fullmatch(r"\s*(\d+)\s*,\s*(\d+)\s*", uc)
        if not mc:
            raise PairError("bad UiCenter= %r (want X,Y pixels)" % uc)
        p.ui_center = (int(mc.group(1)), int(mc.group(2)))
    else:
        p.ui_center = (p.size[0] // 2, p.size[1] // 2)
    p.race = get(items, "Race")
    p.launch = shlex.split(get(items, "Launch", "") or "")
    p.at = []
    for k, v in items:
        if k.lower() == "at":
            mm = re.match(r"(\S+)\s+(.+)$", v)
            if not mm:
                raise PairError("bad At=%r (want '<sim s> <command>')" % v)
            cmd = mm.group(2).strip()
            if ";" in cmd and any(is_rclick(c.strip()) for c in cmd.split(";")):
                raise PairError("At=%r: rclick is a pointer action, not a scene command: give it an At= of its own" % v)
            p.at.append((secs(mm.group(1), "At"), cmd))
    p.scene_sections = [(n, i) for n, i in resolved if n.lower() != "pair"]
    return p


def render_scene_ini(p):
    """The installed Scene.ini: Pair removed, SimTime=1 forced, State=250 default, Race= in [Scene]."""
    secs = [(n, list(i)) for n, i in p.scene_sections]
    names = [n.lower() for n, _ in secs]
    if "scene" not in names:
        secs.insert(0, ("Scene", []))
    if "marks" not in names:
        # after Scene, before objects
        idx = max([i for i, (n, _) in enumerate(secs) if n.lower() == "scene"] or [0]) + 1
        secs.insert(idx, ("Marks", []))
    for n, items in secs:
        ln = n.lower()
        if ln == "scene" and p.race:
            items[:] = [(k, v) for k, v in items if k.lower() != "race"] + [("Race", p.race)]
        if ln == "marks":
            items[:] = [(k, v) for k, v in items if k.lower() != "simtime"] + [("SimTime", "1")]
            if get(items, "State") is None:
                items.append(("State", "250"))
    lines = []
    for n, items in secs:
        if n:
            lines.append("[%s]" % n)
        lines += ["%s=%s" % kv for kv in items]
        lines.append("")
    return "\n".join(lines)


# ------------------------------------------------------------------ marks

def parse_mark(line):
    """-> dict(tick, kind, t (float|None), f (fields), text (str|None)) or None."""
    line = line.rstrip("\r\n")
    m = re.match(r"(\d+)\s+(\S+)(?:\s+(.*))?$", line)
    if not m:
        return None
    rest = m.group(3) or ""
    text = None
    ti = re.search(r"(?:^|\s)text=", rest)
    if ti:
        text = rest[ti.end():]
        rest = rest[:ti.start()]
    fields = {}
    for tok in rest.split():
        if "=" in tok:
            k, v = tok.split("=", 1)
            fields[k] = v
    t = None
    if "t" in fields:
        try:
            t = float(fields["t"])
        except ValueError:
            pass
    return {"tick": int(m.group(1)), "kind": m.group(2), "t": t, "f": fields, "text": text}


def parse_marks(text):
    out = []
    for line in text.splitlines():
        if line.startswith("epoch"):
            continue
        m = parse_mark(line)
        if m:
            out.append(m)
    return out


def ready_t(marks):
    for m in marks:
        if m["kind"] == "ready" and m["t"] is not None:
            return m["t"]
    return None


def latest_t(marks):
    ts = [m["t"] for m in marks if m["t"] is not None]
    return max(ts) if ts else None


def match_commands(marks, sent):
    """For each sent command text (in send order) the t of the first unused
    `cmd` mark with that text, else None. `a; b` is marked command by
    command: the first part's mark lands it, and every part's mark is used
    up so a later command of the same text cannot take one of them."""
    cmds = [m for m in marks if m["kind"] == "cmd"]
    used = [False] * len(cmds)
    res = []
    for text in sent:
        found = None
        start = 0
        for k, part in enumerate(p.strip() for p in text.split(";")):
            for i in range(start, len(cmds)):
                if not used[i] and (cmds[i]["text"] or "").strip() == part:
                    used[i] = True
                    start = i + 1
                    if k == 0:
                        found = cmds[i]["t"]
                    break
            else:
                if k == 0:
                    break
        res.append(found)
    return res


def rel(t, origin):
    return None if t is None or origin is None else round(t - origin, 3)


# ----------------------------------------------------------------- guards

def checkout_containing(path):
    """The git checkout holding bench/pairs that contains path, else None."""
    d = os.path.realpath(path)
    while True:
        if os.path.exists(os.path.join(d, ".git")) and os.path.isdir(os.path.join(d, "bench", "pairs")):
            return d
        parent = os.path.dirname(d)
        if parent == d:
            return None
        d = parent


def is_rclick(cmd):
    """`rclick <object>`: a player's right-click on an object with the current
    selection (`select a b ...` first), through the bench's pointer, not a
    scene command: the order the game's own context logic picks."""
    return cmd.split()[:1] == ["rclick"]


def parse_epochs(text):
    """`epoch <hi> <lo> <tick>` lines: (GetTickCount, unix seconds) pairs."""
    out = []
    for line in text.splitlines():
        w = line.split()
        if len(w) == 4 and w[0] == "epoch":
            try:
                ft = (int(w[1], 16) << 32) | int(w[2], 16)
                out.append((int(w[3]), ft / 1e7 - 11644473600.0))
            except ValueError:
                pass
    return out


def t_at_wall(marks, epochs, wall):
    """The sim time at host wall-clock `wall`: through the latest epoch line
    (GetTickCount <-> host clock) to a tick, then interpolated between the two
    marks with `t=` around it. None when it cannot be placed."""
    if not epochs:
        return None
    tick0, unix0 = epochs[-1]
    tick = tick0 + (wall - unix0) * 1000.0
    pts = sorted((m["tick"], m["t"]) for m in marks if m["t"] is not None)
    before = [x for x in pts if x[0] <= tick]
    after = [x for x in pts if x[0] >= tick]
    if not before or not after:
        return None
    (k0, t0), (k1, t1) = before[-1], after[0]
    if k1 == k0:
        return t0
    return t0 + (t1 - t0) * (tick - k0) / (k1 - k0)


# ---------------------------------------------------------------- the bench

class Bench:
    def __init__(self, refit, log):
        self.refit = refit
        self.a2test = os.path.join(refit, "a2test")
        self.log = log

    def run(self, args, timeout):
        t0 = time.time()
        r = subprocess.run([self.a2test] + args, cwd=self.refit, capture_output=True, text=True, timeout=timeout)
        return (r.stdout or "") + (r.stderr or ""), r.returncode, time.time() - t0


def start_argv(p):
    a = []
    if p.game == "roots":
        a += ["--game", "fo-roots"]
    a += ["session", "start", "--res", "%dx%d" % p.size]
    if p.game == "roots":
        a += ["--layers", "display,asiloader"]
    else:
        a += ["--mod", "stock", "--clock", "bench"]
    a += ["--no-launch", "--label", "pair-%s" % p.game, "--note", "STA64 TOOL-15 paired scene (%s)" % p.game]
    return a


def launch_argv(p, sid):
    return ["drive", "--session", sid, "launch"] + p.launch


def events(p):
    ev = [(t, 0, "at", c) for t, c in p.at]
    ev += [(t, 1, "frame", i) for i, t in enumerate(p.frames)]
    ev.sort(key=lambda e: (e[0], e[1]))  # stable: file order within a time
    return ev


def print_plan(p, scene_ini, out):
    print("=== installed Scene.ini ===")
    print(scene_ini)
    print("=== plan (%s) ===" % p.game)
    print("session start: ./a2test " + " ".join(shlex.quote(x) for x in start_argv(p)))
    print("launch:        ./a2test " + " ".join(shlex.quote(x) for x in launch_argv(p, "<ID>")))
    print("duration:      %g sim s after ready" % p.duration)
    print("timeline:")
    for t, _, kind, x in events(p):
        if kind == "at" and is_rclick(x):
            print("  t=%g  %s: scene 'camera on <target>', pointer to %d,%d, right button" % ((t, x) + p.ui_center))
        elif kind == "at":
            print("  t=%g  scene %s" % (t, x))
        else:
            print("  t=%g  pause, shot pair-f%d -> frames/f%d.png, objects -> objects-f%d.jsonl, resume" % (t, x, x, x))
    print("  t=%g  scene pause, scene objects, gather, session stop" % p.duration)
    print("out:           %s" % out)


class Runner:
    def __init__(self, p, refit, out, timeout):
        self.p, self.refit, self.out, self.timeout = p, refit, out, timeout
        self.deadline = time.time() + timeout
        self.log = open(os.path.join(out, "driver.log"), "a")
        self.bench = Bench(refit, self.log)
        self.sid = None
        self.exe = None
        self.marks_path = None
        self.moff = 0
        self.marks = []
        self.sent = []      # command texts in send order
        self.answers = {}   # index in sent -> answer
        self.notes = []
        self.samples = []   # (wall, sim t) pairs, for the sim rate
        self.latency = 1.0  # wall seconds a drive command takes (moving average)
        self.end_idx = None
        self.ui_wall = {}   # index in sent -> host wall time of a right-click

    def say(self, msg):
        line = "[wall %s] %s" % (time.strftime("%H:%M:%S"), msg)
        self.log.write(line + "\n")
        self.log.flush()
        print(line, flush=True)

    def t_now(self):
        return latest_t(self.marks)

    def remaining(self):
        r = self.deadline - time.time()
        if r <= 0:
            raise PairError("timeout of %gs exceeded" % self.timeout)
        return r

    def poll_marks(self):
        try:
            with open(self.marks_path, "rb") as f:
                f.seek(self.moff)
                data = f.read()
        except OSError:
            return
        nl = data.rfind(b"\n")
        if nl < 0:
            return
        self.moff += nl + 1
        new = parse_marks(data[:nl + 1].decode("utf-8", "replace"))
        self.marks += new
        t = latest_t(new)
        if t is not None:
            self.samples.append((time.time(), t))
            del self.samples[:-200]

    def rate(self):
        """Sim seconds per wall second over the last ~1.5 s (1.0 until known or while paused)."""
        if len(self.samples) < 2:
            return 1.0
        w1, t1 = self.samples[-1]
        for w0, t0 in reversed(self.samples):
            if w1 - w0 >= 1.5:
                break
        if w1 - w0 < 0.5 or t1 <= t0:
            return 1.0
        return max(0.5, min(10.0, (t1 - t0) / (w1 - w0)))

    def scene(self, cmd):
        out, rc, dt = self.bench.run(["drive", "--session", self.sid, "scene", cmd], min(90, self.remaining()))
        self.poll_marks()
        out = out.strip()
        self.latency = 0.5 * self.latency + 0.5 * (dt + 0.25)  # + the marks' 250 ms granularity
        self.sent.append(cmd)
        self.answers[len(self.sent) - 1] = out
        self.say("scene %r (sim t=%s, %.1fs wall, rc=%d) -> %s" % (cmd, self.t_now(), dt, rc, out.replace("\n", " | ")))
        return out

    def rclick(self, cmd):
        """`rclick <object>`: centre the RTS camera on it, then move the
        bench's pointer onto the middle of the view and press the right
        button, as a player does (Roots' shell wants the pointer over a spot
        before it takes a click). Lands when the button went down."""
        target = cmd.split()[1] if len(cmd.split()) > 1 else ""
        self.scene("camera on %s" % target)
        time.sleep(0.6)
        cx, cy = self.p.ui_center
        drive = ["drive", "--session", self.sid]
        self.bench.run(drive + ["move", str(cx + 4), str(cy)], min(30, self.remaining()))
        time.sleep(0.3)
        self.bench.run(drive + ["move", str(cx), str(cy)], min(30, self.remaining()))
        time.sleep(0.3)
        t0 = time.time()
        out, rc, dt = self.bench.run(drive + ["click", str(cx), str(cy), "--button", "3"], min(30, self.remaining()))
        # The press happened while `drive click` ran: take the middle (an
        # uncertainty of half its run, a few tenths of a second).
        wall = (t0 + time.time()) / 2
        self.sent.append(cmd)
        self.answers[len(self.sent) - 1] = out.strip()
        self.ui_wall[len(self.sent) - 1] = wall
        self.poll_marks()
        self.say("%s (sim t~%s, %.1fs wall, rc=%d) -> %s" % (cmd, self.t_now(), dt, rc, out.strip().replace("\n", " | ")))

    def find_files(self):
        sd = os.path.expanduser("~/.cache/a2test/sessions/%s/session.json" % self.sid)
        work = json.load(open(sd))["work"]
        game = os.path.join(work, "game")
        for d in (game, os.path.join(game, "Data")):
            if os.path.isfile(os.path.join(d, "Scene.asi")):
                return d
        raise PairError("no Scene.asi under %s after bench-setup" % game)

    def wait_ready(self):
        end = time.time() + READY_TIMEOUT
        while time.time() < end:
            self.remaining()
            logp = os.path.join(self.exe, "Scene.log")
            try:
                if "scene ready" in open(logp, errors="replace").read():
                    break
            except OSError:
                pass
            time.sleep(2)
        else:
            raise PairError("scene not ready after %gs" % READY_TIMEOUT)
        while ready_t(self.marks) is None:
            self.remaining()
            self.poll_marks()
            if any(m["kind"] == "ready" and m["t"] is None for m in self.marks):
                raise PairError("the ready mark has no t=: this Refit checkout's Scene.asi has no "
                                "[Marks] SimTime (use the bench/measure-marks branch: --refit or $REFIT_ROOT)")
            time.sleep(0.1)

    def wait_t(self, target, lead=False):
        """Return when sim t reaches target; with lead, early by what a command takes
        to land (measured drive latency times the sim rate), so it lands near target.
        A simulation that stops advancing for STALL_WALL seconds (the game paused
        itself: a mission briefing, a menu) fails the run instead of waiting for
        the timeout."""
        seen, since = None, time.time()
        while True:
            self.poll_marks()
            t = self.t_now()
            if t is not None and t + (self.rate() * self.latency if lead else 0.0) >= target:
                return
            if t != seen:
                seen, since = t, time.time()
            elif time.time() - since > STALL_WALL:
                raise PairError("the simulation stopped at t=%s for %ds of wall time (the game paused "
                                "itself? a mission map opens a briefing: use a skirmish or empty map)" % (t, STALL_WALL))
            self.remaining()
            time.sleep(0.1)

    def copy(self, name, dest):
        src = os.path.join(self.exe, name)
        if os.path.exists(src):
            shutil.copyfile(src, dest)
            return True
        self.notes.append("%s missing in the game directory" % name)
        return False

    def stop_new_sessions(self, before):
        """A start that failed part-way may still have made a session: stop
        any session with this driver's label that was not listed before."""
        try:
            now = self.bench.run(["session", "list"], 60)[0]
        except Exception as e:  # noqa: BLE001 -- best effort, say why
            self.say("session list after a failed start: %s" % e)
            return
        old = {l.split()[0] for l in before.splitlines() if l.split()}
        for l in now.splitlines():
            w = l.split()
            if w and w[0] not in old and ("pair-%s" % self.p.game) in l:
                o, rc, _ = self.bench.run(["session", "stop", w[0]], 300)
                self.say("stopped half-started session %s rc=%d: %s" % (w[0], rc, o.strip()[-200:]))

    def go(self, scene_ini):
        p, out = self.p, self.out
        os.makedirs(os.path.join(out, "frames"), exist_ok=True)
        existing = self.bench.run(["session", "list"], 60)[0]
        self.say("session list before start:\n" + existing.strip())
        try:
            o, rc, _ = self.bench.run(start_argv(p), min(900, self.remaining()))
            self.log.write(o + "\n")
            m = re.search(r"^session (\S+)", o, re.M)
            if not m:
                raise PairError("session start gave no session id (rc=%d): %s" % (rc, o[-500:]))
        except BaseException:
            self.stop_new_sessions(existing)
            raise
        self.sid = m.group(1)
        self.say("started session %s" % self.sid)
        try:
            return self._run(scene_ini)
        finally:
            for attempt in (1, 2):
                try:
                    o, rc, _ = self.bench.run(["session", "stop", self.sid], 300)
                except subprocess.TimeoutExpired:
                    self.say("session stop %s timed out (attempt %d)" % (self.sid, attempt))
                    continue
                self.say("session stop %s rc=%d: %s" % (self.sid, rc, o.strip()[-300:]))
                break
            self.say("session list after stop:\n" + self.bench.run(["session", "list"], 60)[0].strip())

    def _run(self, scene_ini):
        p, out = self.p, self.out
        setup = os.path.join(self.refit, "testbench", "scene", "bench-setup.sh")
        statef = os.path.expanduser("~/.cache/a2test/sessions/%s/session.json" % self.sid)
        r = subprocess.run(["bash", setup, statef], cwd=self.refit, capture_output=True, text=True, timeout=min(600, self.remaining()))
        self.say("bench-setup: %s %s" % (r.stdout.strip(), r.stderr.strip()))
        if r.returncode:
            raise PairError("bench-setup.sh failed")
        self.exe = self.find_files()
        with open(os.path.join(self.exe, "Scene.ini"), "w") as f:
            f.write(scene_ini)
        for n in ("Scene.log", "Scene.cmd", "Scene.marks", "Scene.objects.jsonl", "Scene.objects.tmp"):
            try:
                os.unlink(os.path.join(self.exe, n))
            except OSError:
                pass
        self.marks_path = os.path.join(self.exe, "Scene.marks")
        o, rc, _ = self.bench.run(launch_argv(p, self.sid), min(180, self.remaining()))
        self.say("launch %s rc=%d: %s" % (p.launch, rc, o.strip()[-300:]))
        try:
            self.wait_ready()
            r0 = ready_t(self.marks)
            self.say("ready at sim t=%s" % r0)
            frames = []
            for t, _, kind, x in events(p):
                self.wait_t(r0 + t, lead=True)
                if kind == "at" and is_rclick(x):
                    self.rclick(x)
                elif kind == "at":
                    self.scene(x)
                else:
                    frames.append(self.frame(x, t))
            self.wait_t(r0 + p.duration, lead=True)
            self.scene("pause")
            self.end_idx = len(self.sent) - 1
            self.scene("objects")  # paused: the state at the final pause
            time.sleep(0.5)
            self.poll_marks()
        finally:
            for n in ("Scene.ini", "Scene.log", "Scene.marks", "Scene.objects.jsonl"):
                self.copy(n, os.path.join(out, n))
        return self.finish(frames)

    def frame(self, i, at):
        self.scene("pause")
        pause_idx = len(self.sent) - 1
        o, rc, dt = self.bench.run(["drive", "--session", self.sid, "shot", "pair-f%d" % i], min(90, self.remaining()))
        self.say("shot pair-f%d rc=%d -> %s" % (i, rc, o.strip().replace("\n", " | ")))
        png = None
        for line in reversed(o.strip().splitlines()):
            if line.strip().endswith(".png") and os.path.exists(line.strip()):
                png = line.strip()
                break
        file = None
        if png:
            file = "frames/f%d.png" % i
            shutil.copyfile(png, os.path.join(self.out, file))
        else:
            self.notes.append("frame %d: no PNG path in shot output" % i)
        self.scene("objects")
        objs = "objects-f%d.jsonl" % i
        if not self.copy("Scene.objects.jsonl", os.path.join(self.out, objs)):
            objs = None
        self.scene("resume")
        return {"at": at, "pause_idx": pause_idx, "file": file, "objects": objs}

    def finish(self, frames):
        p = self.p
        self.poll_marks()
        r0 = ready_t(self.marks)
        times = match_commands(self.marks, self.sent)
        if self.ui_wall:
            try:
                epochs = parse_epochs(open(self.marks_path, errors="replace").read())
            except OSError:
                epochs = []
            for i, wall in self.ui_wall.items():
                times[i] = t_at_wall(self.marks, [e for e in epochs if e[1] <= wall] or epochs, wall)
                if times[i] is not None:
                    times[i] = round(times[i], 3)
        cmds = []
        idx = 0
        for t, c in p.at:
            # sent order == event order; locate this At by walking self.sent
            while idx < len(self.sent) and self.sent[idx] != c:
                idx += 1
            lt = rel(times[idx], r0) if idx < len(self.sent) else None
            if lt is None:
                self.notes.append("command %r at %g: no cmd mark found" % (c, t))
            cmds.append({"at": t, "text": c, "landed_t": lt, "answer": self.answers.get(idx)})
            idx += 1
        fr = []
        for f in frames:
            pi = f["pause_idx"]
            ft = rel(times[pi], r0)
            m = re.search(r"paused at tick (\d+)", self.answers.get(pi, ""))
            tick = int(m.group(1)) if m else None
            if ft is None:
                self.notes.append("frame at %g: no cmd mark for pause" % f["at"])
            if tick is None:
                self.notes.append("frame at %g: no 'paused at tick N' answer" % f["at"])
            fr.append({"at": f["at"], "t": ft, "tick": tick, "file": f["file"], "objects": f["objects"]})
        end_t = rel(times[self.end_idx], r0) if self.end_idx is not None else None
        if end_t is None:
            end_t = rel(latest_t(self.marks), r0)
            self.notes.append("end_t from the latest mark, not the final pause")
        try:
            commit = subprocess.run(["git", "-C", self.refit, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        except OSError:
            commit = ""
        return {
            "schema": 1, "game": p.game, "session": self.sid, "launch": p.launch,
            "refit_root": self.refit, "refit_commit": commit, "size": list(p.size),
            "ready_t": r0, "duration": p.duration, "end_t": end_t,
            "commands": cmds, "frames": fr, "notes": self.notes,
        }


def default_lock():
    """The machine's one original-game lock: `$STA64_BENCH_LOCK`, else
    `.claude/locks/bench.lock` of the main STA64 checkout (a worktree's
    `.git` file names it), else beside this script's checkout."""
    env = os.environ.get("STA64_BENCH_LOCK")
    if env:
        return env
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    gitfile = os.path.join(root, ".git")
    if os.path.isfile(gitfile):
        # `gitdir: <main>/.git/worktrees/<name>`
        txt = open(gitfile, encoding="utf-8").read().strip()
        if txt.startswith("gitdir:"):
            gd = txt.split(":", 1)[1].strip()
            if not os.path.isabs(gd):
                gd = os.path.normpath(os.path.join(root, gd))
            main = gd.split(os.sep + ".git" + os.sep)[0]
            if os.path.isdir(main):
                root = main
    return os.path.join(root, ".claude", "locks", "bench.lock")


def take_bench_lock(path, wait):
    """One original-game session on the machine at a time: hold an exclusive
    flock on `path` for the whole run (as `flock -w 540`). None when busy."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT, 0o644)
    end = time.monotonic() + wait
    while True:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return fd
        except BlockingIOError:
            if time.monotonic() >= end:
                os.close(fd)
                return None
            time.sleep(1.0)


def main(argv=None):
    ap = argparse.ArgumentParser(description="Run a pair file on the original game (Refit bench).")
    ap.add_argument("--pair", required=True)
    ap.add_argument("--game", required=True, choices=KNOWN_GAMES)
    ap.add_argument("--out", required=True)
    ap.add_argument("--refit", default=os.environ.get("REFIT_ROOT"),
                    help="the Refit checkout (default $REFIT_ROOT); [Marks] SimTime needs its bench/measure-marks branch")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--timeout", type=float, default=600.0, help="wall seconds for the whole run")
    ap.add_argument("--lock", default=None, help="the bench lock file (default $STA64_BENCH_LOCK, else <STA64 checkout>/.claude/locks/bench.lock)")
    ap.add_argument("--lock-wait", type=float, default=540.0, help="seconds to wait for the bench lock (then exit 75, BENCH-BUSY)")
    a = ap.parse_args(argv)
    try:
        p = load_pair(open(a.pair, encoding="utf-8").read(), a.game)
        co = checkout_containing(a.out)
        if co:
            raise PairError("--out %s is inside the git checkout %s; frames of stock art must stay out of the repository" % (a.out, co))
        scene_ini = render_scene_ini(p)
        out = os.path.abspath(a.out)
        if not a.refit and not a.dry_run:
            raise PairError("name the Refit checkout: --refit PATH or $REFIT_ROOT")
        refit = os.path.abspath(a.refit or ".")
        if a.dry_run:
            print_plan(p, scene_ini, out)
            return 0
        for need in (os.path.join(refit, "a2test"), os.path.join(refit, "testbench", "scene", "bench-setup.sh")):
            if not os.path.exists(need):
                raise PairError("not a Refit checkout: %s missing" % need)
    except PairError as e:
        print("refit_side: %s" % e, file=sys.stderr)
        return 2
    import signal

    def _term(signum, frame):
        raise KeyboardInterrupt("signal %d" % signum)
    signal.signal(signal.SIGTERM, _term)
    lock = a.lock or default_lock()
    lfd = take_bench_lock(lock, a.lock_wait)
    if lfd is None:
        print("BENCH-BUSY: another original-game session holds %s; run again" % lock, file=sys.stderr)
        return 75
    os.makedirs(out, exist_ok=True)
    try:
        os.nice(19)
    except OSError:
        pass
    r = Runner(p, refit, out, a.timeout)
    try:
        doc = r.go(scene_ini)
    except (PairError, subprocess.TimeoutExpired) as e:
        r.say("FAILED: %s" % e)
        print("refit_side: %s" % e, file=sys.stderr)
        return 1
    with open(os.path.join(out, "refit.json"), "w") as f:
        json.dump(doc, f, indent=1)
    r.say("wrote refit.json")
    return 0


if __name__ == "__main__":
    sys.exit(main())
