//! The footage log's plain-number parts: the in-frame test, the enter/leave debounce, the fight
//! window and the `game` label. No Bevy and no sim types, so they are tested with plain numbers.

use std::collections::{BTreeMap, BTreeSet};

use sta_content::Profile;

/// A short label for what is being played, to help a cutter sort clips.
///
/// This is a cutting hint, not a rules decision: nothing in the game reads it, and the raw
/// profile and mod are always logged next to it. `Fo4` covers both Fleet Operations 4 and the
/// "Roots" content, so it reads `roots` when any of `mod_names` (the active mod's folder and
/// `[Mod] Name`, the mounted mod layers, and the `[Mod] Name` of every mounted `info.ini`) has
/// "roots" as a whole word ([`has_word`]), else `fo4`.
pub fn game_label(profile: Profile, mod_names: &[&str]) -> &'static str {
    match profile {
        Profile::Sta1Classic => "a1",
        Profile::Sta2Classic => "a2",
        Profile::Fo3 => "dw",
        Profile::Fo4 if mod_names.iter().any(|n| has_word(n, "roots")) => "roots",
        Profile::Fo4 => "fo4",
        Profile::Native => "native",
    }
}

/// Whether `text` has `word` as a whole word, ignoring case. Words split at anything that is not
/// a letter or digit and at a lower-case letter followed by an upper-case one (`FleetOpsRoots`),
/// so "Grassroots" does not match and "Fleet Ops Roots" does.
pub fn has_word(text: &str, word: &str) -> bool {
    let mut cur = String::new();
    let mut prev_lower = false;
    let mut words = Vec::new();
    for c in text.chars() {
        if (!c.is_alphanumeric() || (prev_lower && c.is_uppercase())) && !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
        if c.is_alphanumeric() {
            cur.push(c);
        }
        prev_lower = c.is_lowercase();
    }
    words.push(cur);
    words.iter().any(|w| w.eq_ignore_ascii_case(word))
}

/// Keys reported once until they are released: `shields_down` for a unit's facing is raised on
/// every hit while the shield is down, and the log wants one record per spell.
#[derive(Debug, Clone)]
pub struct Latch<K: Ord + Copy>(BTreeSet<K>);

impl<K: Ord + Copy> Default for Latch<K> {
    fn default() -> Self {
        Latch(BTreeSet::new())
    }
}

impl<K: Ord + Copy> Latch<K> {
    /// `true` the first time `k` is noted since it was last released.
    pub fn note(&mut self, k: K) -> bool {
        self.0.insert(k)
    }

    /// Keeps only the keys `still_held` says so for.
    pub fn retain(&mut self, still_held: impl Fn(&K) -> bool) {
        self.0.retain(|k| still_held(k));
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Where one unit stands in the camera's picture, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projected {
    /// Centre, from the viewport's top left.
    pub x: f32,
    pub y: f32,
    /// Radius of the unit's sphere as it appears.
    pub radius: f32,
    /// The centre is in front of the camera.
    pub in_front: bool,
}

/// A unit counts as in frame when its centre is in front of the camera and its circle (centre
/// grown by the projected radius) overlaps the `w` x `h` viewport. Occlusion is not considered.
pub fn in_frame(p: Projected, w: f32, h: f32) -> bool {
    p.in_front && p.x + p.radius >= 0.0 && p.x - p.radius <= w && p.y + p.radius >= 0.0 && p.y - p.radius <= h
}

/// Pixels a sphere of world `radius` covers at `depth` in front of a camera whose
/// `focal` is pixels per world unit at distance 1.
pub fn projected_radius(radius: f32, depth: f32, focal: f32) -> f32 {
    if depth > 0.0 { radius * focal / depth } else { f32::INFINITY }
}

/// A unit's place in the picture as the log writes it: centre `x`,`y` in 0..1 of the viewport
/// (may fall slightly outside when only the edge of a big unit shows) and `size`, the projected
/// diameter over the viewport height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

pub fn placement(p: Projected, w: f32, h: f32) -> Placement {
    Placement { x: p.x / w.max(1.0), y: p.y / h.max(1.0), size: p.radius * 2.0 / h.max(1.0) }
}

/// One change of who is in frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge<K> {
    Enter(K),
    Leave(K),
}

/// Turns "who is in frame now" samples into `Enter`/`Leave` edges. A key enters at once; it
/// leaves only after it has been out of every sample for `leave_after` seconds, so a ship on the
/// edge of the frame does not make a record for every wobble.
#[derive(Debug, Clone)]
pub struct EdgeTracker<K: Ord + Copy> {
    leave_after: f64,
    /// Known in frame; `Some(t)`: out of the samples since `t`.
    known: BTreeMap<K, Option<f64>>,
}

impl<K: Ord + Copy> EdgeTracker<K> {
    pub fn new(leave_after: f64) -> Self {
        EdgeTracker { leave_after, known: BTreeMap::new() }
    }

    /// Forget everything and take `now_in` as the state, without edges (after a full list was logged).
    pub fn reset(&mut self, now_in: impl IntoIterator<Item = K>) {
        self.known = now_in.into_iter().map(|k| (k, None)).collect();
    }

    #[cfg(test)]
    pub fn is_in(&self, k: &K) -> bool {
        self.known.contains_key(k)
    }

    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    /// Feed a sample taken at `now` seconds. Edges come in key order, leaves before enters.
    pub fn update(&mut self, now: f64, sample: &BTreeSet<K>) -> Vec<Edge<K>> {
        let mut out = Vec::new();
        let mut gone = Vec::new();
        for (k, out_since) in self.known.iter_mut() {
            if sample.contains(k) {
                *out_since = None;
            } else {
                let since = *out_since.get_or_insert(now);
                if now - since >= self.leave_after {
                    gone.push(*k);
                }
            }
        }
        for k in gone {
            self.known.remove(&k);
            out.push(Edge::Leave(k));
        }
        for k in sample {
            if !self.known.contains_key(k) {
                self.known.insert(*k, None);
                out.push(Edge::Enter(*k));
            }
        }
        out
    }
}

/// Who fired or was hit lately, and the on/off edge of "a fight is going on".
///
/// The same rule as the action camera's `FightLog` (a unit is fighting for `window` seconds after
/// a shot fired or taken); kept apart so the log does not depend on that camera's picture.
#[derive(Debug, Clone)]
pub struct FightWatch<K: Ord + Copy> {
    window: f64,
    last: BTreeMap<K, f64>,
    on: bool,
}

impl<K: Ord + Copy> FightWatch<K> {
    pub fn new(window: f64) -> Self {
        FightWatch { window, last: BTreeMap::new(), on: false }
    }

    pub fn note(&mut self, id: K, now: f64) {
        self.last.insert(id, now);
    }

    /// Units fighting at `now` that pass `visible`, ascending.
    pub fn fighting(&mut self, now: f64, visible: impl Fn(&K) -> bool) -> Vec<K> {
        let window = self.window;
        self.last.retain(|_, t| now - *t <= window);
        self.last.keys().copied().filter(|k| visible(k)).collect()
    }

    /// `Some(true)` when a fight starts (some unit fighting), `Some(false)` when the last one stops.
    pub fn edge(&mut self, fighting: usize) -> Option<bool> {
        let now_on = fighting > 0;
        (now_on != std::mem::replace(&mut self.on, now_on)).then_some(now_on)
    }

    pub fn clear(&mut self) {
        self.last.clear();
        self.on = false;
    }

    /// A fight is on, as far as the last [`edge`](Self::edge) said.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Marks the fight over without an edge (the caller wrote the `off` record).
    pub fn close(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_label_is_a_hint_from_the_profile_and_the_mod() {
        assert_eq!(game_label(Profile::Sta1Classic, &[]), "a1");
        assert_eq!(game_label(Profile::Sta2Classic, &["Roots"]), "a2");
        assert_eq!(game_label(Profile::Fo3, &[]), "dw");
        assert_eq!(game_label(Profile::Native, &[]), "native");
        assert_eq!(game_label(Profile::Fo4, &[]), "fo4");
        assert_eq!(game_label(Profile::Fo4, &["Fleet Ops 4.0"]), "fo4");
        assert_eq!(game_label(Profile::Fo4, &["fo-ROOTS-1"]), "roots");
        assert_eq!(game_label(Profile::Fo4, &["Data", "Fleet Operations Roots"]), "roots");
        assert_eq!(game_label(Profile::Fo4, &["FleetOpsRoots"]), "roots");
        assert_eq!(game_label(Profile::Fo4, &["Grassroots Edition"]), "fo4");
        assert_eq!(game_label(Profile::Fo4, &["Rootsmod"]), "fo4");
    }

    fn at(x: f32, y: f32, radius: f32) -> Projected {
        Projected { x, y, radius, in_front: true }
    }

    #[test]
    fn a_unit_is_in_frame_when_its_circle_overlaps_the_viewport() {
        assert!(in_frame(at(100.0, 100.0, 5.0), 200.0, 100.0));
        // The centre is off screen, the circle still reaches in.
        assert!(in_frame(at(-4.0, 50.0, 5.0), 200.0, 100.0));
        assert!(in_frame(at(203.0, 50.0, 5.0), 200.0, 100.0));
        // Just clear of the edge.
        assert!(!in_frame(at(-6.0, 50.0, 5.0), 200.0, 100.0));
        assert!(!in_frame(at(100.0, 106.0, 5.0), 200.0, 100.0));
        assert!(!in_frame(at(100.0, -6.0, 5.0), 200.0, 100.0));
        // A big near unit that covers the whole picture.
        assert!(in_frame(at(-500.0, 50.0, 2000.0), 200.0, 100.0));
        // Behind the camera, whatever the numbers say.
        assert!(!in_frame(Projected { in_front: false, ..at(100.0, 50.0, 5.0) }, 200.0, 100.0));
    }

    #[test]
    fn radius_shrinks_with_depth_and_is_infinite_behind() {
        assert_eq!(projected_radius(10.0, 100.0, 500.0), 50.0);
        assert_eq!(projected_radius(10.0, 200.0, 500.0), 25.0);
        assert!(projected_radius(10.0, 0.0, 500.0).is_infinite());
    }

    #[test]
    fn placement_is_normalised_to_the_viewport() {
        let p = placement(at(480.0, 135.0, 27.0), 960.0, 540.0);
        assert_eq!((p.x, p.y, p.size), (0.5, 0.25, 0.1));
    }

    fn set(v: &[u32]) -> BTreeSet<u32> {
        v.iter().copied().collect()
    }

    #[test]
    fn enter_is_at_once_and_leave_waits_out_the_debounce() {
        let mut t = EdgeTracker::new(0.5);
        assert_eq!(t.update(0.0, &set(&[1, 2])), [Edge::Enter(1), Edge::Enter(2)]);
        assert_eq!(t.update(0.2, &set(&[1, 2])), []);
        // 2 leaves the sample at 0.4; it is not gone until 0.5 s later.
        assert_eq!(t.update(0.4, &set(&[1])), []);
        assert_eq!(t.update(0.8, &set(&[1])), []);
        assert_eq!(t.update(0.9, &set(&[1])), [Edge::Leave(2)]);
        assert!(!t.is_in(&2) && t.is_in(&1));
    }

    #[test]
    fn a_unit_on_the_edge_makes_no_spam() {
        let mut t = EdgeTracker::new(0.5);
        t.update(0.0, &set(&[7]));
        let mut edges = 0;
        for i in 1..40 {
            let now = f64::from(i) * 0.2;
            // In and out every other sample, never out for 0.5 s.
            let sample = if i % 2 == 0 { set(&[7]) } else { set(&[]) };
            edges += t.update(now, &sample).len();
        }
        assert_eq!(edges, 0);
        // Out for good: exactly one leave.
        let mut leaves = 0;
        for i in 0..10 {
            leaves += t.update(10.0 + f64::from(i) * 0.2, &set(&[])).len();
        }
        assert_eq!(leaves, 1);
    }

    #[test]
    fn reset_takes_the_state_without_edges() {
        let mut t = EdgeTracker::new(0.5);
        t.reset([3, 4]);
        assert_eq!(t.update(0.0, &set(&[3, 4])), []);
        assert_eq!(t.update(0.1, &set(&[3, 4, 5])), [Edge::Enter(5)]);
    }

    #[test]
    fn a_latch_reports_once_until_released() {
        let mut l = Latch::default();
        assert!(l.note((1u32, 0u8)));
        assert!(!l.note((1, 0)));
        assert!(l.note((1, 1)));
        // Facing 0 is back up; facing 1 is still down.
        l.retain(|k| k.1 == 1);
        assert!(l.note((1, 0)));
        assert!(!l.note((1, 1)));
    }

    fn step(f: &mut FightWatch<u32>, now: f64) -> Option<bool> {
        let n = f.fighting(now, |_| true).len();
        f.edge(n)
    }

    #[test]
    fn a_fight_is_on_for_the_window_after_the_last_shot() {
        let mut f = FightWatch::new(6.0);
        assert_eq!(step(&mut f, 0.0), None);
        f.note(1u32, 1.0);
        f.note(2, 2.0);
        assert_eq!(step(&mut f, 2.0), Some(true));
        assert_eq!(step(&mut f, 3.0), None);
        // 1 expires at 7.0, 2 at 8.0.
        assert_eq!(f.fighting(7.5, |_| true), [2]);
        assert_eq!(f.edge(1), None);
        assert!(f.fighting(8.5, |_| true).is_empty());
        assert_eq!(f.edge(0), Some(false));
        assert_eq!(f.edge(0), None);
    }

    #[test]
    fn only_visible_units_count_as_fighting() {
        let mut f = FightWatch::new(6.0);
        f.note(1u32, 0.0);
        f.note(2, 0.0);
        assert_eq!(f.fighting(1.0, |k| *k == 2), [2]);
        assert!(f.fighting(1.0, |_| false).is_empty());
    }
}
