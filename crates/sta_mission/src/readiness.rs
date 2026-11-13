//! Campaign readiness probe (LUA-05): for every mission of an installed
//! campaign book, does its script resolve, load and run on the mission's own
//! map as [`CampaignRunner`] starts it, and which of its API uses are unbound
//! or bound-but-unsupported. Nothing here reads a game file itself; it drives
//! [`VfsHost`] and the runtime's own counters. `tools/campaign_readiness.py`
//! prints the table (docs/MISSIONS.md, "Campaign readiness").
//!
//! Evidence grade: measured on the runtime (a run of N ticks, no player input).
//! A script that ends the mission itself before N ticks stops early.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use crate::campaign::*;

/// Prefixes of the runtime's permissive fallback (`stub global <name>`,
/// `stub field <path>`): a name the API does not bind at all.
const STUB_PREFIX: &str = "stub ";

/// Split the runtime's `unsupported()` list into names no binding exists for
/// (unbound) and calls that are bound but raise or ignore the request.
pub fn classify(counted: &[(String, u32)]) -> (Vec<(String, u32)>, Vec<(String, u32)>) {
    counted.iter().cloned().partition(|(k, _)| k.starts_with(STUB_PREFIX))
}

/// What one mission did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MissionProbe {
    pub campaign: String,
    pub index: usize,
    pub bzn: String,
    /// `lua`, `legacy` or `none`.
    pub kind: String,
    /// The Lua path, or the legacy files joined with `+`.
    pub script: String,
    /// Legacy DLL files (`.dsl`, `.drl`) when `kind` is `legacy`.
    pub files: Vec<String>,
    /// The runner started it (script loaded without error, map built).
    pub started: bool,
    pub start_error: Option<String>,
    pub ticks: u64,
    /// `Some(won)` when the script ended the mission within the horizon.
    pub ended: Option<bool>,
    pub errors: Vec<String>,
    pub unbound: Vec<(String, u32)>,
    pub unsupported: Vec<(String, u32)>,
}

impl MissionProbe {
    /// (a) the mission resolves to a Lua script.
    pub fn resolves(&self) -> bool {
        self.kind == "lua"
    }
    /// (b) loaded and ran the horizon (or ended itself) without an error.
    pub fn runs(&self) -> bool {
        self.resolves() && self.started && self.errors.is_empty()
    }
    /// (c) runs and touched no unbound name.
    pub fn bound(&self) -> bool {
        self.runs() && self.unbound.is_empty()
    }
    /// (d) bound and nothing it called is unsupported.
    pub fn working(&self) -> bool {
        self.bound() && self.unsupported.is_empty()
    }

    /// One JSON object on one line (no serde dependency).
    pub fn to_json(&self) -> String {
        fn s(v: &str) -> String {
            let mut o = String::from("\"");
            for c in v.chars() {
                match c {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    '\n' => o.push_str("\\n"),
                    '\r' => o.push_str("\\r"),
                    '\t' => o.push_str("\\t"),
                    c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                    c => o.push(c),
                }
            }
            o.push('"');
            o
        }
        fn list(v: &[String]) -> String {
            format!("[{}]", v.iter().map(|x| s(x)).collect::<Vec<_>>().join(","))
        }
        fn counted(v: &[(String, u32)]) -> String {
            format!("[{}]", v.iter().map(|(k, n)| format!("[{},{}]", s(k), n)).collect::<Vec<_>>().join(","))
        }
        let ended = match self.ended {
            Some(true) => "true",
            Some(false) => "false",
            None => "null",
        };
        let start_error = self.start_error.as_deref().map(s).unwrap_or_else(|| "null".into());
        format!(
            "{{\"campaign\":{},\"index\":{},\"bzn\":{},\"kind\":{},\"script\":{},\"files\":{},\"started\":{},\"start_error\":{},\"ticks\":{},\"ended\":{},\"errors\":{},\"unbound\":{},\"unsupported\":{}}}",
            s(&self.campaign),
            self.index,
            s(&self.bzn),
            s(&self.kind),
            s(&self.script),
            list(&self.files),
            self.started,
            start_error,
            self.ticks,
            ended,
            list(&self.errors),
            counted(&self.unbound),
            counted(&self.unsupported),
        )
    }
}

/// Probe one mission with a fresh runner (every campaign open, as with
/// `GIVE_ALL_MISSIONS`), running up to `ticks` ticks.
pub fn probe_mission(host: &VfsHost, book: &CampaignBook, campaign: &str, index: usize, ticks: u64) -> MissionProbe {
    let m = &book.campaigns.iter().find(|c| c.id == campaign).expect("campaign in book").missions[index];
    let (kind, script, files) = match &m.script {
        MissionScript::Lua { path } => ("lua", path.clone(), Vec::new()),
        MissionScript::LegacyDll { files } => ("legacy", files.join("+"), files.clone()),
        MissionScript::None => ("none", String::new(), Vec::new()),
    };
    let mut p = MissionProbe {
        campaign: campaign.to_owned(),
        index,
        bzn: m.bzn.clone(),
        kind: kind.into(),
        script,
        files,
        ..Default::default()
    };
    if kind != "lua" {
        return p;
    }
    let mut open = book.clone();
    open.give_all_missions = true;
    let mut progress = CampaignProgress::new(&open);
    progress.apply_give_all(&open);
    let mut runner = CampaignRunner::with_progress(open, progress);
    let run = catch_unwind(AssertUnwindSafe(|| match runner.start(host, campaign, index, 1) {
        Err(e) => Err(e.to_string()),
        Ok(a) => {
            let ended = a.run(ticks);
            let rt = a.runtime();
            Ok((a.sim().tick, ended.map(|o| o.won), rt.errors().to_vec(), rt.unsupported()))
        }
    }));
    match run {
        Err(_) => {
            p.start_error = Some("panic while running".into());
        }
        Ok(Err(e)) => p.start_error = Some(e),
        Ok(Ok((tick, ended, errors, counted))) => {
            p.started = true;
            p.ticks = tick;
            p.ended = ended;
            p.errors = errors;
            let (unbound, unsupported) = classify(&counted);
            p.unbound = unbound;
            p.unsupported = unsupported;
        }
    }
    p
}

/// Probe every mission of `book`.
pub fn probe_book(host: &VfsHost, book: &CampaignBook, ticks: u64) -> Vec<MissionProbe> {
    let mut out = Vec::new();
    for c in &book.campaigns {
        for m in &c.missions {
            out.push(probe_mission(host, book, &c.id, m.index, ticks));
        }
    }
    out
}

/// Mount `root` (with `active_mod` over it), load its campaign book and probe it.
pub fn probe_install(root: &std::path::Path, active_mod: Option<&str>, ticks: u64) -> Result<Vec<MissionProbe>, String> {
    let vfs = sta_vfs::Vfs::mount_install(root, active_mod).map_err(|e| format!("mount: {e}"))?;
    let book = CampaignBook::load_vfs(&vfs);
    let host = VfsHost::load(Arc::new(vfs))?;
    Ok(probe_book(&host, &book, ticks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_splits_stubs_from_bound_unsupported() {
        let counted = vec![
            ("A1.SetWarpSpeed: no seam".to_owned(), 2),
            ("stub field Foo.bar".to_owned(), 1),
            ("stub global Baz".to_owned(), 3),
            ("Player.loadAIP".to_owned(), 1),
        ];
        let (unbound, unsupported) = classify(&counted);
        assert_eq!(unbound.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["stub field Foo.bar", "stub global Baz"]);
        assert_eq!(unsupported.len(), 2);
    }

    #[test]
    fn funnel_is_monotone() {
        let mut p = MissionProbe { kind: "lua".into(), started: true, ..Default::default() };
        assert!(p.working());
        p.unsupported.push(("x".into(), 1));
        assert!(p.bound() && !p.working());
        p.unbound.push(("stub global y".into(), 1));
        assert!(p.runs() && !p.bound());
        p.errors.push("boom".into());
        assert!(p.resolves() && !p.runs());
        p.kind = "legacy".into();
        assert!(!p.resolves());
    }

    #[test]
    fn json_escapes_and_nulls() {
        let p = MissionProbe { campaign: "c\"x".into(), kind: "none".into(), errors: vec!["a\nb".into()], ..Default::default() };
        let j = p.to_json();
        assert!(j.contains("\"campaign\":\"c\\\"x\""));
        assert!(j.contains("\"errors\":[\"a\\nb\"]"));
        assert!(j.contains("\"ended\":null") && j.contains("\"start_error\":null"));
    }
}
