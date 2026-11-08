//! GUI layout files (`misc/gui_*.cfg`, CON-12): typed layouts for the HUD
//! and menus (`docs/data/cfg.md`).
//!
//! A GUI CFG is `key = value` lines with `//` comments and `#include
//! "file.cfg"`. Values are numbers (a rectangle is `x y w h`, a colour `r g b`
//! in 0..1), quoted sprite or cursor names, or flags. Coordinates are in the
//! file's canvas, `screenWidth` × `screenHeight` (Armada I: 640 × 480).
//!
//! **Which value wins.** A key is looked up in the file's own lines first,
//! then in its includes in the order written, depth first; the first hit
//! wins, and each file is read once. Both games need exactly this:
//!
//! - Armada II `gui_bor.cfg` includes `gui_interface.cfg` at its top, then
//!   sets its own `noLatinum` and resource panels; the Borg have no latinum,
//!   so a file's own lines beat its includes.
//! - Fleet Operations `gui_federation16x9.cfg` is `#include "gui_16x9.cfg"`
//!   then `#include "gui_federation.cfg"` (which reaches `gui_interface.cfg`).
//!   `gui_16x9.cfg` alone sets `screenWidth = 2134` and places `buttonPanelArea`
//!   at x 1857 (fits 2134); `gui_interface.cfg` has x 1323 (fits 1600). So the
//!   earlier include beats the later one.
//!
//! "Last assignment wins" (as ODFs work) gets the second case wrong.

use std::collections::{BTreeMap, BTreeSet};

use crate::Warning;

/// One effective value and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiValue {
    /// Key as written.
    pub key: String,
    /// Value text, comment removed, trimmed.
    pub raw: String,
    /// File (as named by the include, or the root name) and 1-based line.
    pub file: String,
    pub line: usize,
}

/// `x y w h` in canvas units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// From canvas units to a `screen` of another size, scaled per axis.
    pub fn scaled(self, canvas: (i32, i32), screen: (i32, i32)) -> [f32; 4] {
        let sx = screen.0 as f32 / canvas.0.max(1) as f32;
        let sy = screen.1 as f32 / canvas.1.max(1) as f32;
        [self.x as f32 * sx, self.y as f32 * sy, self.w as f32 * sx, self.h as f32 * sy]
    }
}

/// `r g b` or `r g b a`, each 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colour {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// One HUD or menu panel: everything keyed `<name>PanelArea`, `<name>BackgroundPanel…`,
/// `<name>DefaultCursor`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Panel {
    /// The prefix as first written (`minimap`, `buildBox`).
    pub name: String,
    /// `<name>PanelArea`.
    pub area: Option<Rect>,
    /// `<name>PanelArea_<n>`: alternative areas (info panel heights, ...).
    pub variants: BTreeMap<u32, Rect>,
    /// `<name>BackgroundPanel`: the sprite (SPR entry) drawn behind it.
    pub background: Option<String>,
    pub background_area: Option<Rect>,
    /// `<name>BackgroundPanelSize`: how many `_n` pieces the background has.
    pub background_size: Option<i64>,
    /// `<name>BackgroundPanel_<n>`: the background's pieces.
    pub pieces: BTreeMap<u32, Rect>,
    pub cursor: Option<String>,
}

/// A whole resolved layout, typed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GuiLayout {
    /// `screenWidth` × `screenHeight`, or the default given to [`GuiCfg::layout`].
    pub canvas: (i32, i32),
    /// Panels by lower-case name.
    pub panels: BTreeMap<String, Panel>,
    /// Every other rectangle (`timeLineArea`, `selectSystem_3`, ...), by key as written.
    pub rects: BTreeMap<String, Rect>,
    /// Colours, by key as written.
    pub colours: BTreeMap<String, Colour>,
    /// Quoted names (sprites, cursors, fonts) not owned by a panel.
    pub names: BTreeMap<String, String>,
    /// Plain numbers and flags (`noLatinum`, `popupPaletteXA`).
    pub numbers: BTreeMap<String, f64>,
}

impl GuiLayout {
    /// A stable, line-per-fact dump for snapshot tests and diffs.
    pub fn dump(&self) -> String {
        let r = |r: &Rect| format!("{} {} {} {}", r.x, r.y, r.w, r.h);
        let mut out = format!("canvas {} {}\n", self.canvas.0, self.canvas.1);
        for p in self.panels.values() {
            out += &format!("panel {}", p.name);
            if let Some(a) = &p.area {
                out += &format!(" area {}", r(a));
            }
            out += "\n";
            for (n, v) in &p.variants {
                out += &format!("  variant {n} {}\n", r(v));
            }
            if let Some(b) = &p.background {
                out += &format!("  background {b:?}");
                if let Some(a) = &p.background_area {
                    out += &format!(" area {}", r(a));
                }
                if let Some(s) = p.background_size {
                    out += &format!(" size {s}");
                }
                out += "\n";
            }
            for (n, v) in &p.pieces {
                out += &format!("  piece {n} {}\n", r(v));
            }
            if let Some(c) = &p.cursor {
                out += &format!("  cursor {c:?}\n");
            }
        }
        for (k, v) in &self.rects {
            out += &format!("rect {k} {}\n", r(v));
        }
        for (k, c) in &self.colours {
            out += &format!("colour {k} {} {} {} {}\n", c.r, c.g, c.b, c.a);
        }
        for (k, v) in &self.names {
            out += &format!("name {k} {v:?}\n");
        }
        for (k, v) in &self.numbers {
            out += &format!("number {k} {v}\n");
        }
        out
    }
}

/// A resolved GUI CFG: every key's effective value, plus the files read.
#[derive(Debug, Clone, Default)]
pub struct GuiCfg {
    pub name: String,
    /// Lower-case key -> effective value.
    values: BTreeMap<String, GuiValue>,
    /// Files in the order they were read.
    files: Vec<String>,
}

struct Parsed {
    entries: Vec<(String, String, usize)>,
    includes: Vec<(String, usize)>,
}

fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    let b = line.as_bytes();
    for i in 0..b.len() {
        match b[i] {
            b'"' => in_str = !in_str,
            b'/' if !in_str && b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse(text: &str) -> Parsed {
    let mut p = Parsed { entries: Vec::new(), includes: Vec::new() };
    for (i, line) in text.lines().enumerate() {
        let l = strip_comment(line).trim();
        if l.is_empty() {
            continue;
        }
        if let Some(rest) = l.strip_prefix('#') {
            let rest = rest.trim_start();
            if rest.get(..7).is_some_and(|w| w.eq_ignore_ascii_case("include")) {
                let name = rest[7..].trim().trim_matches(['"', '<', '>']).trim();
                if !name.is_empty() {
                    p.includes.push((name.to_owned(), i + 1));
                }
            }
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            let k = k.trim();
            if !k.is_empty() && !k.contains(char::is_whitespace) {
                p.entries.push((k.to_owned(), v.trim().to_owned(), i + 1));
            }
        }
    }
    p
}

fn ints(raw: &str) -> Option<Vec<i64>> {
    raw.split_whitespace().map(|t| t.trim_end_matches(';').parse::<i64>().ok()).collect()
}

fn floats(raw: &str) -> Option<Vec<f64>> {
    raw.split_whitespace().map(|t| t.trim_end_matches(['f', 'F', ';']).parse::<f64>().ok()).collect()
}

fn unquote(raw: &str) -> Option<&str> {
    raw.strip_prefix('"').and_then(|s| s.split_once('"')).map(|(s, _)| s)
}

/// `name_12` -> (`name`, 12).
fn numbered(key: &str) -> Option<(&str, u32)> {
    let (base, n) = key.rsplit_once('_')?;
    Some((base, n.parse().ok()?))
}

impl GuiCfg {
    /// Resolve `name` and its includes. `load` maps an include name (as
    /// written, e.g. `"gui_interface.cfg"`) to its text. Missing includes,
    /// include loops and keys set twice in one file are warnings.
    pub fn load(name: &str, load: &mut dyn FnMut(&str) -> Option<String>) -> (GuiCfg, Vec<Warning>) {
        let mut cfg = GuiCfg { name: name.to_owned(), ..Default::default() };
        let mut warnings = Vec::new();
        let mut read = BTreeSet::new();
        cfg.apply(name, load, &mut read, &mut Vec::new(), &mut warnings, None);
        (cfg, warnings)
    }

    /// One file with no includes available.
    pub fn parse(name: &str, text: &str) -> (GuiCfg, Vec<Warning>) {
        let mut first = Some(text.to_owned());
        Self::load(name, &mut |n| if n == name { first.take() } else { None })
    }

    fn apply(
        &mut self,
        name: &str,
        load: &mut dyn FnMut(&str) -> Option<String>,
        read: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
        warnings: &mut Vec<Warning>,
        from: Option<(&str, usize)>,
    ) {
        let key = name.to_ascii_lowercase();
        if stack.contains(&key) {
            warnings.push(Warning::new(name, format!("include loop: {} -> {name}", stack.join(" -> "))));
            return;
        }
        if !read.insert(key.clone()) {
            return; // already read through another include
        }
        let Some(text) = load(name) else {
            let w = match from {
                Some((f, line)) => Warning::new(f, format!("line {line}: include not found: {name}")),
                None => Warning::new(name, "not found"),
            };
            warnings.push(w);
            return;
        };
        self.files.push(name.to_owned());
        let p = parse(&text);
        // Own lines first: the first assignment of a key in this file counts.
        let mut own: BTreeMap<String, usize> = BTreeMap::new();
        for (k, v, line) in p.entries {
            let lower = k.to_ascii_lowercase();
            if let Some(first) = own.get(&lower) {
                if self.values.get(&lower).is_some_and(|e| e.raw != v) {
                    warnings.push(Warning::new(name, format!("{k} set again on line {line} (line {first} counts)")));
                }
                continue;
            }
            own.insert(lower.clone(), line);
            self.values.entry(lower).or_insert(GuiValue { key: k, raw: v, file: name.to_owned(), line });
        }
        stack.push(key);
        for (inc, line) in &p.includes {
            self.apply(inc, load, read, stack, warnings, Some((name, *line)));
        }
        stack.pop();
    }

    /// Files read, in order (the root first).
    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn get(&self, key: &str) -> Option<&GuiValue> {
        self.values.get(&key.to_ascii_lowercase())
    }

    pub fn raw(&self, key: &str) -> Option<&str> {
        self.get(key).map(|v| v.raw.as_str())
    }

    /// Every effective value, by lower-case key.
    pub fn values(&self) -> impl Iterator<Item = &GuiValue> {
        self.values.values()
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        ints(self.raw(key)?)?.first().copied()
    }

    pub fn number(&self, key: &str) -> Option<f64> {
        floats(self.raw(key)?)?.first().copied()
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        unquote(self.raw(key)?)
    }

    pub fn rect(&self, key: &str) -> Option<Rect> {
        match ints(self.raw(key)?)?.as_slice() {
            [x, y, w, h] => Some(Rect { x: *x as i32, y: *y as i32, w: *w as i32, h: *h as i32 }),
            _ => None,
        }
    }

    pub fn colour(&self, key: &str) -> Option<Colour> {
        match floats(self.raw(key)?)?.as_slice() {
            [r, g, b] => Some(Colour { r: *r as f32, g: *g as f32, b: *b as f32, a: 1.0 }),
            [r, g, b, a] => Some(Colour { r: *r as f32, g: *g as f32, b: *b as f32, a: *a as f32 }),
            _ => None,
        }
    }

    /// `screenWidth` × `screenHeight`, when set.
    pub fn canvas(&self) -> Option<(i32, i32)> {
        let w = self.int("screenWidth").filter(|v| *v > 0)?;
        let h = self.int("screenHeight").filter(|v| *v > 0)?;
        Some((w as i32, h as i32))
    }

    /// The typed layout. `default_canvas` applies when the files set no
    /// `screenWidth`/`screenHeight` (Armada I: 640 × 480).
    pub fn layout(&self, default_canvas: (i32, i32)) -> GuiLayout {
        let mut l = GuiLayout { canvas: self.canvas().unwrap_or(default_canvas), ..Default::default() };
        let mut owned: BTreeSet<String> = ["screenwidth", "screenheight"].into_iter().map(String::from).collect();

        // Panels: every `<name>PanelArea[_n]` except the background's own area.
        for v in self.values.values() {
            let lower = v.key.to_ascii_lowercase();
            let (base, n) = match numbered(&lower) {
                Some((b, n)) if b.ends_with("panelarea") => (b, Some(n)),
                _ => (lower.as_str(), None),
            };
            let Some(prefix) = base.strip_suffix("panelarea") else { continue };
            if prefix.is_empty() || prefix.ends_with("background") || prefix.ends_with("minimized") {
                continue;
            }
            let Some(rect) = self.rect(&v.key) else { continue };
            let name = v.key[..prefix.len()].to_owned();
            let p = l.panels.entry(prefix.to_owned()).or_insert_with(|| Panel { name, ..Default::default() });
            match n {
                None => p.area = Some(rect),
                Some(n) => {
                    p.variants.insert(n, rect);
                }
            }
            owned.insert(lower.clone());
        }
        for (prefix, p) in l.panels.iter_mut() {
            let k = |s: &str| format!("{prefix}{s}");
            if let Some(b) = self.str(&k("backgroundpanel")) {
                p.background = Some(b.to_owned());
                owned.insert(k("backgroundpanel"));
            }
            if let Some(r) = self.rect(&k("backgroundpanelarea")) {
                p.background_area = Some(r);
                owned.insert(k("backgroundpanelarea"));
            }
            if let Some(n) = self.int(&k("backgroundpanelsize")) {
                p.background_size = Some(n);
                owned.insert(k("backgroundpanelsize"));
            }
            if let Some(c) = self.str(&k("defaultcursor")) {
                p.cursor = Some(c.to_owned());
                owned.insert(k("defaultcursor"));
            }
            let piece = k("backgroundpanel_");
            for (lower, v) in self.values.range(piece.clone()..) {
                let Some(n) = lower.strip_prefix(&piece) else { break };
                if let (Ok(n), Some(r)) = (n.parse::<u32>(), self.rect(&v.key)) {
                    p.pieces.insert(n, r);
                    owned.insert(lower.clone());
                }
            }
        }

        // Everything else, by shape.
        for (lower, v) in &self.values {
            if owned.contains(lower) {
                continue;
            }
            let is_colour = lower.contains("color") || lower.contains("colour");
            if let Some(s) = unquote(&v.raw) {
                l.names.insert(v.key.clone(), s.to_owned());
            } else if is_colour && let Some(c) = self.colour(&v.key) {
                l.colours.insert(v.key.clone(), c);
            } else if let Some(r) = self.rect(&v.key) {
                l.rects.insert(v.key.clone(), r);
            } else if let Some(n) = floats(&v.raw).filter(|f| f.len() == 1) {
                l.numbers.insert(v.key.clone(), n[0]);
            }
        }
        l
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &'static str) -> impl FnMut(&str) -> Option<String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/gui").join(dir);
        move |name| {
            let want = name.to_ascii_lowercase();
            std::fs::read_dir(&root)
                .ok()?
                .flatten()
                .find(|e| e.file_name().to_string_lossy().to_ascii_lowercase() == want)
                .and_then(|e| std::fs::read_to_string(e.path()).ok())
        }
    }

    #[test]
    fn own_lines_beat_includes() {
        // The A2 Borg case.
        let (cfg, w) = GuiCfg::load("gui_bor.cfg", &mut fixture("a2"));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg.int("noLatinum"), Some(1));
        assert_eq!(cfg.rect("resourcePanelArea"), Some(Rect { x: 600, y: 0, w: 400, h: 24 }));
        assert_eq!(cfg.get("noLatinum").unwrap().file, "gui_bor.cfg");
        assert_eq!(cfg.get("minimapPanelArea").unwrap().file, "gui_interface.cfg");
        assert_eq!(cfg.files(), ["gui_bor.cfg", "gui_interface.cfg", "gui_glob16x12.cfg"]);
    }

    #[test]
    fn earlier_include_beats_later() {
        // The FO widescreen case.
        let (cfg, w) = GuiCfg::load("gui_federation16x9.cfg", &mut fixture("fo"));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg.canvas(), Some((2134, 1200)));
        assert_eq!(cfg.rect("buttonPanelArea").unwrap().x, 1857, "16x9 over gui_interface");
        assert_eq!(cfg.rect("minimapPanelArea").unwrap().x, -1, "from gui_interface");
        assert_eq!(cfg.colour("SelectionRectangleColor").map(|c| c.b), Some(1.0));
        // gui_interface.cfg is reached through both race files but read once.
        assert_eq!(cfg.files().iter().filter(|f| f.eq_ignore_ascii_case("gui_interface.cfg")).count(), 1);
    }

    #[test]
    fn layout_snapshot() {
        let (cfg, _) = GuiCfg::load("gui_federation16x9.cfg", &mut fixture("fo"));
        let want = "\
canvas 2134 1200
panel button area 1857 8 266 40
  cursor \"standard_cursor\"
panel info area 0 900 600 300
  variant 1 0 1000 600 200
  variant 2 0 1100 600 100
panel minimap area -1 818 351 383
  background \"minimapBackgroundPanel\" area 0 0 0 0 size 2
  piece 0 0 0 351 383
  piece 1 10 10 20 20
  cursor \"standard_cursor\"
rect infoBarOffset_0 892 50 0 0
colour SelectionRectangleColor 0.4 0.6 1 1
colour tooltipTextColor 1 1 1 1
name minimapBackgroundSprite \"map_background\"
number noLatinum 0
number popupPaletteXA 264
";
        assert_eq!(cfg.layout((640, 480)).dump(), want);
    }

    #[test]
    fn a1_default_canvas_and_scaling() {
        let (cfg, _) = GuiCfg::parse("gui_glob.cfg", "dropPlayerPanelArea = 192 32 256 320\nselectSystem_3 = 0 0 9 9\n");
        let l = cfg.layout((640, 480));
        assert_eq!(l.canvas, (640, 480));
        assert_eq!(l.rects["selectSystem_3"], Rect { x: 0, y: 0, w: 9, h: 9 });
        let r = l.panels["dropplayer"].area.unwrap();
        assert_eq!(r.scaled((640, 480), (1280, 960)), [384.0, 64.0, 512.0, 640.0]);
    }

    #[test]
    fn warnings() {
        let mut load = |n: &str| match n {
            "a.cfg" => Some("#include \"b.cfg\"\n#include \"gone.cfg\"\nx = 1\nx = 2\n".to_string()),
            "b.cfg" => Some("#include \"a.cfg\"\ny = 3\n".to_string()),
            _ => None,
        };
        let (cfg, w) = GuiCfg::load("a.cfg", &mut load);
        assert_eq!((cfg.int("x"), cfg.int("y")), (Some(1), Some(3)));
        let msgs: Vec<_> = w.iter().map(|w| w.to_string()).collect();
        assert!(msgs.iter().any(|m| m.contains("x set again on line 4 (line 3 counts)")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("include not found: gone.cfg")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("include loop")), "{msgs:?}");
    }
}
