//! Frame metrics for paired scenes (TOOL-17; docs/TESTBENCH.md, "Paired
//! scenes", "Frame metrics").
//!
//! Two frames of one scene from two renderers never agree pixel for pixel:
//! the original game's rasteriser, filtering and projection differ in the
//! last pixel, so a plain per-pixel difference is dominated by edges a pixel
//! apart. Alongside it these figures are robust to a small misalignment:
//!
//! - **plain**: mean and largest absolute difference over all channels, the
//!   share of pixels that differ at all and by more than 16 ([`FrameDiff`]);
//! - **blurred** (`midfreq`, `lowfreq`): the root-mean-square difference of the
//!   two frames after a box blur of radius `w/320` and `w/80` pixels (4 and
//!   16 at 1280 wide), beside the unblurred `rms`: what is left when edges are
//!   allowed to be off by a few pixels. A shifted edge is a thin one-signed
//!   pulse of difference, which a blur spreads and so lowers in energy
//!   (about `1/sqrt(2r+1)`), while a wrong colour over an area stays whole.
//!   (The mean *absolute* difference would not do: blurring conserves it.)
//! - **best shift** (`shift`): the smallest mean absolute difference of the
//!   luminance over every whole-pixel offset up to `w/320` (and which offset),
//!   i.e. the plain difference once a fixed misalignment is taken out;
//! - **colour and luminance**: the whole frame's mean colour and luminance of
//!   each side and their difference (`lum`, `colour`), the same over four
//!   regions (`bg`: pixels the original shows dark, `fg`: the rest, `centre`:
//!   the middle half of each axis, `border`: the rest) and over a 4 x 4 grid
//!   of tiles (`tile`: the largest tile difference in mean luminance), plus the
//!   share of pixels each side shows bright (`fgshare`).
//!
//! Luminance is `0.299 R + 0.587 G + 0.114 B` over the 8-bit sRGB values
//! (not linear light). Regions are cut from the **original's** frame (the
//! reference) and applied to both, so a region compares what is drawn in the
//! same place. Every figure is a plain function of the two images: the same
//! frames give the same numbers.

use std::fmt::Write as _;

use crate::pair::{FrameDiff, Image, frame_diff, resize};
use crate::run::json;

/// Luminance above which a pixel counts as bright (object, nebula, planet)
/// rather than dark sky.
pub const BRIGHT: f64 = 48.0;

/// Mean colour and luminance of a set of pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Colour {
    pub rgb: [f64; 3],
    pub lum: f64,
}

/// One named region, on both sides.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub name: &'static str,
    /// Share of the frame's pixels in the region (0..1).
    pub share: f64,
    pub orig: Colour,
    pub sta: Colour,
    /// Mean absolute difference over the region's pixels (0..255).
    pub diff: f64,
}

impl Region {
    /// `|mean luminance of STA64 - mean luminance of the original|`.
    pub fn lum_delta(&self) -> f64 {
        (self.sta.lum - self.orig.lum).abs()
    }
}

/// Everything measured on one frame pair.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameMetrics {
    pub plain: FrameDiff,
    /// Root-mean-square difference, no blur (0..255).
    pub rms: f64,
    pub midfreq: f64,
    pub lowfreq: f64,
    /// The best offset `(dx, dy)` of STA64's frame (pixels, STA64 sampled at
    /// `x + dx`) and the mean absolute luminance difference there.
    pub shift: (i32, i32),
    pub shift_mean: f64,
    pub whole: Region,
    pub regions: Vec<Region>,
    /// Largest `|difference of mean luminance|` over the 4 x 4 tiles.
    pub tile_max: f64,
    /// Share of pixels brighter than [`BRIGHT`]: original, STA64.
    pub bright: (f64, f64),
}

impl FrameMetrics {
    /// `|STA64 mean luminance - original mean luminance|`, whole frame.
    pub fn lum_delta(&self) -> f64 {
        self.whole.lum_delta()
    }

    /// Largest channel difference of the whole-frame mean colour.
    pub fn colour_delta(&self) -> f64 {
        (0..3).map(|k| (self.whole.sta.rgb[k] - self.whole.orig.rgb[k]).abs()).fold(0.0, f64::max)
    }

    pub fn region(&self, name: &str) -> Option<&Region> {
        self.regions.iter().find(|r| r.name == name)
    }

    /// Difference of the bright shares (0..1).
    pub fn bright_delta(&self) -> f64 {
        (self.bright.1 - self.bright.0).abs()
    }
}

/// Opt-in limits per metric; `None` is reported, not judged. Keys of
/// `Tol=` in a pair file and of `--tol`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MetricTol {
    pub midfreq: Option<f64>,
    pub lowfreq: Option<f64>,
    pub shift: Option<f64>,
    pub lum: Option<f64>,
    pub colour: Option<f64>,
    pub bg: Option<f64>,
    pub fg: Option<f64>,
    pub tile: Option<f64>,
    pub bright: Option<f64>,
}

impl MetricTol {
    /// The keys this reads, for the error message.
    pub const KEYS: &'static str = "midfreq, lowfreq, shift, lum, colour, bg, fg, tile, bright";

    /// Set `key`; false when it is not a metric key.
    pub fn set(&mut self, key: &str, v: f64) -> bool {
        let slot = match key {
            "midfreq" => &mut self.midfreq,
            "lowfreq" => &mut self.lowfreq,
            "shift" => &mut self.shift,
            "lum" => &mut self.lum,
            "colour" | "color" => &mut self.colour,
            "bg" => &mut self.bg,
            "fg" => &mut self.fg,
            "tile" => &mut self.tile,
            "bright" => &mut self.bright,
            _ => return false,
        };
        *slot = Some(v);
        true
    }

    /// `key=limit` for each metric that is judged.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for (k, v) in self.pairs() {
            if let Some(v) = v {
                let _ = write!(s, ",{k}={v}");
            }
        }
        s
    }

    fn pairs(&self) -> [(&'static str, Option<f64>); 9] {
        [
            ("midfreq", self.midfreq),
            ("lowfreq", self.lowfreq),
            ("shift", self.shift),
            ("lum", self.lum),
            ("colour", self.colour),
            ("bg", self.bg),
            ("fg", self.fg),
            ("tile", self.tile),
            ("bright", self.bright),
        ]
    }

    /// The metrics of `m` that exceed their limit: `(key, value, limit)`.
    pub fn exceeded(&self, m: &FrameMetrics) -> Vec<(&'static str, f64, f64)> {
        let value = |k: &str| -> f64 {
            match k {
                "midfreq" => m.midfreq,
                "lowfreq" => m.lowfreq,
                "shift" => m.shift_mean,
                "lum" => m.lum_delta(),
                "colour" => m.colour_delta(),
                "bg" => m.region("bg").map_or(0.0, Region::lum_delta),
                "fg" => m.region("fg").map_or(0.0, Region::lum_delta),
                "tile" => m.tile_max,
                _ => m.bright_delta(),
            }
        };
        self.pairs().into_iter().filter_map(|(k, lim)| lim.map(|l| (k, value(k), l))).filter(|(_, v, l)| v > l).collect()
    }
}

fn lum(p: &[u8]) -> f64 {
    0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])
}

/// Box blur of one channel plane, radius `r` pixels, window clamped to the
/// image (so the edge averages what is there).
fn blur_plane(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0f32; w * h];
    let mut pre = vec![0f64; w + 1];
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        for x in 0..w {
            pre[x + 1] = pre[x] + f64::from(row[x]);
        }
        for x in 0..w {
            let (lo, hi) = (x.saturating_sub(r), (x + r + 1).min(w));
            tmp[y * w + x] = ((pre[hi] - pre[lo]) / (hi - lo) as f64) as f32;
        }
    }
    let mut out = vec![0f32; w * h];
    pre = vec![0f64; h + 1];
    for x in 0..w {
        for y in 0..h {
            pre[y + 1] = pre[y] + f64::from(tmp[y * w + x]);
        }
        for y in 0..h {
            let (lo, hi) = (y.saturating_sub(r), (y + r + 1).min(h));
            out[y * w + x] = ((pre[hi] - pre[lo]) / (hi - lo) as f64) as f32;
        }
    }
    out
}

/// The three channel planes of `img`, blurred by `r` (0: unblurred).
fn planes(img: &Image, r: usize) -> [Vec<f32>; 3] {
    let (w, h) = (img.w as usize, img.h as usize);
    let split = |k: usize| -> Vec<f32> { img.rgb.chunks_exact(3).map(|p| f32::from(p[k])).collect() };
    [0, 1, 2].map(|k| {
        let p = split(k);
        if r == 0 { p } else { blur_plane(&p, w, h, r) }
    })
}

/// Root-mean-square difference of the two images after a box blur of radius
/// `r` (`0`: none), over all channels, 0..255. `b` must be the size of `a`.
pub fn blurred_diff(a: &Image, b: &Image, r: usize) -> f64 {
    let (pa, pb) = (planes(a, r), planes(b, r));
    let n = (a.w as usize * a.h as usize * 3).max(1);
    let sum: f64 = (0..3).map(|k| pa[k].iter().zip(&pb[k]).map(|(x, y)| f64::from((x - y) * (x - y))).sum::<f64>()).sum();
    (sum / n as f64).sqrt()
}

/// The blurred difference as an image (largest channel, times `gain`, red).
pub fn blurred_diff_image(a: &Image, b: &Image, r: usize, gain: f32) -> Image {
    let (pa, pb) = (planes(a, r), planes(b, r));
    let mut rgb = Vec::with_capacity(a.rgb.len());
    for i in 0..(a.w * a.h) as usize {
        let d = (0..3).map(|k| (pa[k][i] - pb[k][i]).abs()).fold(0.0, f32::max);
        rgb.extend([(d * gain).min(255.0) as u8, 0, 0]);
    }
    Image { w: a.w, h: a.h, rgb }
}

/// Best whole-pixel offset up to `max` (both axes) of `b` against `a`, by the
/// mean absolute luminance difference over the overlap: `((dx, dy), mean)`.
pub fn best_shift(a: &Image, b: &Image, max: i32) -> ((i32, i32), f64) {
    let (w, h) = (a.w as i32, a.h as i32);
    let la: Vec<f32> = a.rgb.chunks_exact(3).map(|p| lum(p) as f32).collect();
    let lb: Vec<f32> = b.rgb.chunks_exact(3).map(|p| lum(p) as f32).collect();
    let mut best: ((i32, i32), f64) = ((0, 0), f64::MAX);
    let any_overlap = a.w as i32 > max && a.h as i32 > max;
    for dy in -max..=max {
        for dx in -max..=max {
            let (x0, x1) = ((-dx).max(0), (w - dx.max(0)).max(0));
            let (y0, y1) = ((-dy).max(0), (h - dy.max(0)).max(0));
            let n = i64::from((x1 - x0).max(0)) * i64::from((y1 - y0).max(0));
            if n == 0 {
                continue;
            }
            let mut sum = 0f64;
            for y in y0..y1 {
                let ra = &la[(y * w + x0) as usize..(y * w + x1) as usize];
                let rb = &lb[((y + dy) * w + x0 + dx) as usize..((y + dy) * w + x1 + dx) as usize];
                sum += ra.iter().zip(rb).map(|(p, q)| f64::from((p - q).abs())).sum::<f64>();
            }
            let mean = sum / n as f64;
            // Ties go to the smaller offset, so identical frames report (0, 0).
            let key = (mean, dx.abs() + dy.abs());
            let cur = (best.1, best.0.0.abs() + best.0.1.abs());
            if key.0 < cur.0 - 1e-9 || ((key.0 - cur.0).abs() <= 1e-9 && key.1 < cur.1) {
                best = ((dx, dy), mean);
            }
        }
    }
    if !any_overlap || best.1 == f64::MAX { ((0, 0), 0.0) } else { best }
}

struct Acc {
    n: u64,
    rgb: [[f64; 3]; 2],
    diff: f64,
}

impl Acc {
    fn new() -> Acc {
        Acc { n: 0, rgb: [[0.0; 3]; 2], diff: 0.0 }
    }

    fn add(&mut self, pa: &[u8], pb: &[u8]) {
        self.n += 1;
        for k in 0..3 {
            self.rgb[0][k] += f64::from(pa[k]);
            self.rgb[1][k] += f64::from(pb[k]);
            self.diff += f64::from(pa[k].abs_diff(pb[k]));
        }
    }

    fn region(&self, name: &'static str, total: u64) -> Region {
        let n = self.n.max(1) as f64;
        let col = |s: &[f64; 3]| {
            let rgb = [s[0] / n, s[1] / n, s[2] / n];
            Colour { rgb, lum: 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2] }
        };
        Region { name, share: self.n as f64 / total.max(1) as f64, orig: col(&self.rgb[0]), sta: col(&self.rgb[1]), diff: self.diff / (n * 3.0) }
    }
}

/// Every metric of one frame pair. `b` is brought to `a`'s size first (as
/// [`frame_diff`] does); `a` is the original (the reference).
pub fn frame_metrics(a: &Image, b: &Image) -> FrameMetrics {
    let b = resize(b, a.w, a.h);
    let (w, h) = (a.w as usize, a.h as usize);
    let plain = frame_diff(a, &b);
    let total = (w * h) as u64;
    let mut whole = Acc::new();
    let (mut bg, mut fg, mut centre, mut border) = (Acc::new(), Acc::new(), Acc::new(), Acc::new());
    let mut tiles: Vec<Acc> = (0..16).map(|_| Acc::new()).collect();
    let mut bright = (0u64, 0u64);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 3;
            let (pa, pb) = (&a.rgb[i..i + 3], &b.rgb[i..i + 3]);
            whole.add(pa, pb);
            if lum(pa) > BRIGHT { fg.add(pa, pb) } else { bg.add(pa, pb) }
            bright.0 += u64::from(lum(pa) > BRIGHT);
            bright.1 += u64::from(lum(pb) > BRIGHT);
            let inside = x >= w / 4 && x < w * 3 / 4 && y >= h / 4 && y < h * 3 / 4;
            if inside { centre.add(pa, pb) } else { border.add(pa, pb) }
            tiles[(y * 4 / h.max(1)).min(3) * 4 + (x * 4 / w.max(1)).min(3)].add(pa, pb);
        }
    }
    let tile_max = tiles.iter().filter(|t| t.n > 0).map(|t| t.region("tile", total).lum_delta()).fold(0.0, f64::max);
    let reach = (w / 320).max(1);
    let (shift, shift_mean) = best_shift(a, &b, reach as i32);
    FrameMetrics {
        plain,
        rms: blurred_diff(a, &b, 0),
        midfreq: blurred_diff(a, &b, reach),
        lowfreq: blurred_diff(a, &b, (w / 80).max(2)),
        shift,
        shift_mean,
        whole: whole.region("whole", total),
        regions: vec![bg.region("bg", total), fg.region("fg", total), centre.region("centre", total), border.region("border", total)],
        tile_max,
        bright: (bright.0 as f64 / total.max(1) as f64, bright.1 as f64 / total.max(1) as f64),
    }
}

/// `original | STA64 | difference x4 | blurred difference x4`, side by side.
pub fn strip(a: &Image, b: &Image) -> Image {
    let b = resize(b, a.w, a.h);
    let low = blurred_diff_image(a, &b, (a.w as usize / 80).max(2), 4.0);
    let w = a.w * 4;
    let mut rgb = Vec::with_capacity((w * a.h * 3) as usize);
    for y in 0..a.h as usize {
        let r = y * a.w as usize * 3..(y + 1) * a.w as usize * 3;
        rgb.extend(&a.rgb[r.clone()]);
        rgb.extend(&b.rgb[r.clone()]);
        for (pa, pb) in a.rgb[r.clone()].chunks_exact(3).zip(b.rgb[r.clone()].chunks_exact(3)) {
            let d = (0..3).map(|k| pa[k].abs_diff(pb[k])).max().unwrap_or(0);
            rgb.extend([d.saturating_mul(4), 0, 0]);
        }
        rgb.extend(&low.rgb[r]);
    }
    Image { w, h: a.h, rgb }
}

fn num(v: f64) -> String {
    json::num((v * 1000.0).round() / 1000.0)
}

fn colour_json(c: &Colour) -> String {
    format!("{{\"rgb\":[{},{},{}],\"lum\":{}}}", num(c.rgb[0]), num(c.rgb[1]), num(c.rgb[2]), num(c.lum))
}

fn region_json(r: &Region) -> String {
    format!("{{\"share\":{},\"original\":{},\"sta64\":{},\"lum_delta\":{},\"mean_diff\":{}}}", num(r.share), colour_json(&r.orig), colour_json(&r.sta), num(r.lum_delta()), num(r.diff))
}

/// The `metrics` object of one frame in `report.json`.
pub fn metrics_json(m: &FrameMetrics) -> String {
    let mut s = format!(
        "{{\"rms\":{},\"midfreq\":{},\"lowfreq\":{},\"shift\":{{\"dx\":{},\"dy\":{},\"mean\":{}}},\"lum_delta\":{},\"colour_delta\":{},\"tile_max\":{},\"bright\":{{\"original\":{},\"sta64\":{},\"delta\":{}}},\"whole\":{}",
        num(m.rms),
        num(m.midfreq),
        num(m.lowfreq),
        m.shift.0,
        m.shift.1,
        num(m.shift_mean),
        num(m.lum_delta()),
        num(m.colour_delta()),
        num(m.tile_max),
        num(m.bright.0),
        num(m.bright.1),
        num(m.bright_delta()),
        region_json(&m.whole)
    );
    s += ",\"regions\":{";
    for (i, r) in m.regions.iter().enumerate() {
        let _ = write!(s, "{}\"{}\":{}", if i > 0 { "," } else { "" }, r.name, region_json(r));
    }
    s += "}}";
    s
}

/// The metric table of `report.md`: one row per frame (`rows`: label and
/// metrics), then one block of regions per frame.
pub fn metrics_md(rows: &[(String, &FrameMetrics)]) -> String {
    let mut s = String::from(
        "\n### Frame metrics\n\nRobust to a small misalignment: `rms` is the root-mean-square difference, `mid`/`low` the same after a box blur (radius w/320, w/80), `shift` the best whole-pixel offset (STA64 sampled at x+dx, y+dy) with its mean luminance difference, `dLum`/`dCol` the difference of the whole frame's mean luminance and (largest channel) mean colour, `bg`/`fg` the same luminance difference over the pixels the original shows dark / bright, `tile` the largest of a 4 x 4 grid, `bright` the share of bright pixels (original / STA64). Luminance is 0.299R+0.587G+0.114B over sRGB values.\n\n| frame | rms | mid | low | shift | shift mean | dLum | dCol | bg dLum | fg dLum | tile | bright orig | bright STA64 |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for (name, m) in rows {
        let _ = writeln!(
            s,
            "| {name} | {:.2} | {:.2} | {:.2} | {},{} | {:.2} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1}% | {:.1}% |",
            m.rms,
            m.midfreq,
            m.lowfreq,
            m.shift.0,
            m.shift.1,
            m.shift_mean,
            m.lum_delta(),
            m.colour_delta(),
            m.region("bg").map_or(0.0, Region::lum_delta),
            m.region("fg").map_or(0.0, Region::lum_delta),
            m.tile_max,
            m.bright.0 * 100.0,
            m.bright.1 * 100.0
        );
    }
    s += "\nMean colour and luminance by region (original -> STA64):\n\n| frame | region | share | original RGB | STA64 RGB | original lum | STA64 lum | mean diff |\n|---|---|---|---|---|---|---|---|\n";
    for (name, m) in rows {
        for r in std::iter::once(&m.whole).chain(&m.regions) {
            let c = |c: &Colour| format!("{:.0},{:.0},{:.0}", c.rgb[0], c.rgb[1], c.rgb[2]);
            let _ = writeln!(s, "| {name} | {} | {:.0}% | {} | {} | {:.1} | {:.1} | {:.2} |", r.name, r.share * 100.0, c(&r.orig), c(&r.sta), r.orig.lum, r.sta.lum, r.diff);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dark frame with one bright square at `(x0, y0)`, `side` pixels.
    fn square(w: u32, h: u32, x0: u32, y0: u32, side: u32, colour: [u8; 3]) -> Image {
        let mut rgb = vec![10u8; (w * h * 3) as usize];
        for y in y0..(y0 + side).min(h) {
            for x in x0..(x0 + side).min(w) {
                let i = ((y * w + x) * 3) as usize;
                rgb[i..i + 3].copy_from_slice(&colour);
            }
        }
        Image { w, h, rgb }
    }

    #[test]
    fn identical_frames_agree_everywhere() {
        let a = square(64, 48, 20, 10, 12, [200, 180, 160]);
        let m = frame_metrics(&a, &a);
        assert_eq!((m.plain.mean, m.rms, m.midfreq, m.lowfreq, m.shift, m.shift_mean, m.tile_max), (0.0, 0.0, 0.0, 0.0, (0, 0), 0.0, 0.0));
        assert_eq!((m.lum_delta(), m.colour_delta(), m.bright_delta()), (0.0, 0.0, 0.0));
        assert!(MetricTol { lowfreq: Some(0.0), lum: Some(0.0), ..Default::default() }.exceeded(&m).is_empty());
    }

    #[test]
    fn a_one_pixel_shift_barely_moves_the_blurred_difference() {
        let (w, h) = (320, 180);
        let a = square(w, h, 100, 60, 40, [220, 200, 180]);
        let b = square(w, h, 101, 60, 40, [220, 200, 180]);
        let m = frame_metrics(&a, &b);
        // The plain difference sees the two one-pixel-wide edges; the blurred ones see little,
        // and the best shift takes the offset out altogether.
        assert!(m.rms > 5.0, "{}", m.rms);
        assert!(m.lowfreq < m.rms * 0.45, "low {} rms {}", m.lowfreq, m.rms);
        assert!(m.midfreq < m.rms * 0.75, "mid {} rms {}", m.midfreq, m.rms);
        assert_eq!(m.shift, (1, 0));
        assert!(m.shift_mean < 1e-6, "{}", m.shift_mean);
        // Colour and luminance of the frame, of the regions and of the tiles are unchanged by it.
        assert!(m.lum_delta() < 0.05 && m.colour_delta() < 0.05 && m.tile_max < 2.0, "{} {} {}", m.lum_delta(), m.colour_delta(), m.tile_max);
    }

    #[test]
    fn a_different_colour_shows_in_regions_and_bright_share() {
        let (w, h) = (160, 90);
        let a = square(w, h, 60, 30, 40, [200, 60, 60]);
        let b = square(w, h, 60, 30, 40, [60, 60, 200]);
        let m = frame_metrics(&a, &b);
        // Same place, same size, other hue: the background agrees, the object does not.
        assert!(m.region("bg").unwrap().lum_delta() < 0.01);
        assert!(m.region("fg").unwrap().lum_delta() > 20.0, "{}", m.region("fg").unwrap().lum_delta());
        assert!(m.colour_delta() > 10.0);
        assert!(m.region("fg").unwrap().orig.rgb[0] > 190.0 && m.region("fg").unwrap().sta.rgb[2] > 190.0);
        // A missing object: bright share and the tiles say so.
        let empty = square(w, h, 0, 0, 0, [0, 0, 0]);
        let m = frame_metrics(&a, &empty);
        assert!(m.bright_delta() > 0.1 && m.tile_max > 30.0 && m.region("centre").unwrap().lum_delta() > 10.0);
    }

    #[test]
    fn tolerances_judge_only_what_is_set() {
        let mut t = MetricTol::default();
        assert!(t.set("lowfreq", 1.0) && t.set("color", 5.0) && !t.set("frobnicate", 1.0));
        assert_eq!(t.text(), ",lowfreq=1,colour=5");
        let a = square(64, 48, 10, 10, 20, [255, 255, 255]);
        let b = square(64, 48, 10, 10, 20, [0, 0, 0]);
        let m = frame_metrics(&a, &b);
        let over = t.exceeded(&m);
        assert_eq!(over.iter().map(|o| o.0).collect::<Vec<_>>(), ["lowfreq", "colour"]);
        assert!(MetricTol::default().exceeded(&m).is_empty());
    }

    #[test]
    fn the_strip_has_four_panels_and_the_json_parses() {
        let a = square(40, 30, 5, 5, 10, [255, 255, 255]);
        let s = strip(&a, &a);
        assert_eq!((s.w, s.h), (160, 30));
        let m = frame_metrics(&a, &a);
        let j = metrics_json(&m);
        let v = crate::pair::parse_json(&j).expect("json");
        assert!(v.get("regions").get("bg").get("share").num().is_some());
        assert!(metrics_md(&[("f0".into(), &m)]).contains("| f0 |"));
    }
}
