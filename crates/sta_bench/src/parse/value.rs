//! Small value types and token helpers shared by the three parsers.

/// A time in the bench's two units.
///
/// Refit counts the old engine's ticks, which are as long as the frame that ran
/// them, so cross-engine scenes use seconds (`Pause=3.2s`). STA64's own scenes
/// may count ticks. The parser keeps whichever was written; the runner turns
/// seconds into ticks with [`TimeSpec::ticks`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeSpec {
    Ticks(u64),
    Seconds(f64),
}

impl TimeSpec {
    /// `200` is ticks, `3.2s` (or `3.2 s`) is seconds. A fractional number of
    /// ticks, a negative one or a non-finite one is not a time.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some(secs) = s.strip_suffix(['s', 'S']) {
            let v = number(secs.trim_end())?;
            return (v >= 0.0).then_some(Self::Seconds(v));
        }
        let v = number(s)?;
        (v >= 0.0 && v.fract() == 0.0 && v <= u64::MAX as f64).then_some(Self::Ticks(v as u64))
    }

    /// Whole ticks at `hz` ticks a second; seconds round to nearest.
    pub fn ticks(self, hz: u32) -> u64 {
        match self {
            Self::Ticks(n) => n,
            Self::Seconds(s) => (s * f64::from(hz)).round() as u64,
        }
    }
}

/// Something a command points at: a point, or an object by name (`#3` for the
/// sim's id). Refit tells them apart the same way: three numbers are a point.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Point([f64; 3]),
    Object(String),
}

/// An engine system a scene or command can switch (`Hud=0`, `hud off`).
///
/// Refit patches the old engine for each; STA64 answers `! unsupported:
/// <system>` for any whose system has not landed yet, and the scene still loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum System {
    Fog,
    Hud,
    Grid,
    Cursor,
    Notices,
    Tooltips,
    Hover,
    ShieldFx,
    Music,
    Captions,
}

impl System {
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key.to_ascii_lowercase().as_str() {
            "fog" => Self::Fog,
            "hud" => Self::Hud,
            "grid" => Self::Grid,
            "cursor" => Self::Cursor,
            "notices" => Self::Notices,
            "tooltips" => Self::Tooltips,
            "hover" => Self::Hover,
            "shieldfx" => Self::ShieldFx,
            "music" => Self::Music,
            "captions" => Self::Captions,
            _ => return None,
        })
    }

    /// The lower-case name used in `! unsupported: <system>` answers.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fog => "fog",
            Self::Hud => "hud",
            Self::Grid => "grid",
            Self::Cursor => "cursor",
            Self::Notices => "notices",
            Self::Tooltips => "tooltips",
            Self::Hover => "hover",
            Self::ShieldFx => "shieldfx",
            Self::Music => "music",
            Self::Captions => "captions",
        }
    }
}

/// A finite number. Refit's own reader (`s_atof`) takes the leading digits of
/// anything, so it reads `Pause=3.2s` as tick 3; here a plain number must be
/// one, and a typo is an error.
pub(crate) fn number(s: &str) -> Option<f64> {
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Refit's `on_arg` accepts `on`/`1` and reads everything else as off, which
/// turns a typo into "off". Here only the four spellings are accepted.
pub(crate) fn on_off(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "1" => Some(true),
        "off" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_specs() {
        assert_eq!(TimeSpec::parse("200"), Some(TimeSpec::Ticks(200)));
        assert_eq!(TimeSpec::parse("3.2s"), Some(TimeSpec::Seconds(3.2)));
        assert_eq!(TimeSpec::parse("3.2 s"), Some(TimeSpec::Seconds(3.2)));
        assert_eq!(TimeSpec::parse("1.5"), None);
        assert_eq!(TimeSpec::parse("-1"), None);
        assert_eq!(TimeSpec::parse("s"), None);
        assert_eq!(TimeSpec::parse("nan"), None);
        assert_eq!(TimeSpec::Seconds(3.2).ticks(30), 96);
        assert_eq!(TimeSpec::Ticks(7).ticks(30), 7);
    }
}
