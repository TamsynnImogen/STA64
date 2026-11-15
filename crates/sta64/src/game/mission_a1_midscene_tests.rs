//! Install-gated (`STA64_TEST_A1_INSTALL`, needs `python3` and `objdump`): the middle of Armada I
//! Federation 1, the Premonition scene. The opening is run as in `mission_a1_opening_tests`, then
//! the state the scene waits for (both stations built, every Jem'Hadar destroyed) is given to the
//! sim as bench edits, and the scene runs on its own, headless.
//!
//! No window, no audio device. Set `STA64_TRACE_OUT` to a file to get one line per frame.

use super::a1_opening::{drive_mission, secs};
use super::*;
use crate::input::camera_cinematic::CinematicCamera;
use crate::ui::presentation::{CineractiveState, FadeState, PresentationLog};
use sta_core::{Fixed, PlayerId};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::present::Presentation;
use std::fmt::Write as _;

pub(super) struct Sample {
    pub tick: u64,
    pub cine: bool,
    pub locked: bool,
    pub fade: f32,
    pub premo: Option<(Vec3, f32)>,
    pub borg: Vec<(Vec3, f32)>,
    pub line: String,
}

pub(super) struct Scene {
    pub samples: Vec<Sample>,
    pub log: Vec<(u64, Presentation)>,
}

fn lua_state(app: &App, names: &[&str]) -> String {
    let rt = app.world().resource::<MissionHandle>();
    let mut chunk = String::from("local s = {} ");
    for n in names {
        let _ = write!(chunk, "s[#s+1] = '{n}=' .. tostring(Mission_.state['{n}']) ");
    }
    chunk.push_str("local t = {} for _, v in ipairs(Mission_.timers) do t[#t+1] = v.name .. ':' .. string.format('%.2f', v.remaining) end ");
    chunk.push_str("local w = {} for _, sc in ipairs(Mission_.scripts) do w[#w+1] = sc.name .. '@' .. tostring(sc.names[sc.at]) end ");
    chunk.push_str("return table.concat(s, ' ') .. ' timers[' .. table.concat(t, ',') .. '] scripts[' .. table.concat(w, ',') .. ']'");
    rt.with(|r| r.eval::<String>(&chunk).unwrap_or_else(|e| format!("eval error {e}")))
}

/// Runs the opening, gives the sim what the Premonition scene waits for, and runs `seconds` of
/// game time more (or until the mission ends), sampling every frame.
pub(super) fn drive_scene(seconds: f64) -> Option<Scene> {
    let run = drive_mission(58.0, "fed1.bzn")?;
    let mut app = run.app;
    assert!(!app.world().resource::<CineractiveState>().on, "the opening is over before the scene is entered");
    let content = app.world_mut().resource_scope(|w, mut cat: Mut<crate::map::Catalogue>| cat.content(&w.resource::<crate::viewer::Mounted>().0).expect("content").db.clone());
    let key = |n: &str| content.key(n).unwrap_or_else(|| panic!("{n} is in the install"));
    let (dom, prem, borg_def, ent, base, mine) = (key("zjemdest"), key("fprem"), key("bcruise1"), key("fente"), key("fbase"), key("fmining"));
    // The hook: the Jem'Hadar destroyed and both stations standing, in the same tick.
    let view = app.world().resource::<SimLink>().curr().clone();
    let at = view.units.iter().find(|u| u.def == ent).map(|u| u.pos).expect("the Enterprise exists");
    let mut seq = 1_000_000u32;
    let mut cmds = Vec::new();
    for u in view.units.iter().filter(|u| u.def == dom) {
        seq += 1;
        cmds.push(Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(Edit::Kill(u.id)) });
    }
    for (odf, dx) in [(base, 300), (mine, -300)] {
        seq += 1;
        let pos = sta_core::Vec3 { x: at.x + Fixed::from_int(dx), y: at.y, z: at.z };
        cmds.push(Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(Edit::Spawn { odf, player: PlayerId(0), pos, heading: (Fixed::ZERO, Fixed::ONE) }) });
    }
    {
        let mut link = app.world_mut().resource_mut::<SimLink>();
        for c in cmds {
            link.submit_as(c);
        }
    }
    let vars = ["PREPREMO", "PREMOCINCHECK", "WAITINGFORPREM", "WAITINGFORBORG", "CUTTOWIDE", "PREMOEXISTS", "BORGEXISTS", "ENDCONDITIONCHECK", "WINCINCHECK", "LOSECHECK", "isCineractiveDone", "isCineractiveCancel"];
    let mut samples = Vec::new();
    let start = app.world().resource::<SimLink>().curr().tick;
    let mut last_log = 0usize;
    let mut out = String::new();
    for (n, pts) in &app.world().resource::<crate::input::camera_cinematic::ScenePaths>().0 {
        if ["WORMENTER", "WORMEXIT", "PREMOCREATE", "CREATEBORG1", "CREATEBORG2", "WORMSHOT", "PREMOSHOT", "PREMOSTART", "PREMOATTACK1", "PREMOATTACK2", "ENTSTART", "ENTPAN"].contains(&n.as_str()) {
            let _ = writeln!(out, "path {n}: {:?}", pts.iter().take(3).collect::<Vec<_>>());
        }
    }
    for _ in 0..40_000 {
        app.update();
        let w = app.world();
        let link = w.resource::<SimLink>();
        let tick = link.curr().tick;
        let o = w.get_resource::<CinematicCamera>().map(|c| c.locked).unwrap_or(false);
        let (eye, focus) = {
            let mut q = app.world_mut().query::<&crate::viewer::Orbit>();
            let orb = q.single(app.world()).unwrap();
            (crate::input::camera_cinematic::camera_translation(&crate::input::camera_cinematic::Pose { focus: orb.focus, yaw: orb.yaw, pitch: orb.pitch, dist: orb.dist }), orb.focus)
        };
        let w = app.world();
        let view = w.resource::<SimLink>().curr().clone();
        let to_bevy = |p: sta_core::Vec3| crate::render::units::to_bevy(p.to_f32());
        let premo = view.units.iter().find(|u| u.def == prem).map(|u| (to_bevy(u.pos), u.speed.to_f32()));
        let hull = view.units.iter().find(|u| u.def == prem).map(|u| (u.hull[0].to_f32(), u.hull[1].to_f32(), u.shields.iter().map(|s| s[0].to_f32()).sum::<f32>(), u.shields.iter().map(|s| s[1].to_f32()).sum::<f32>()));
        let borg: Vec<(Vec3, f32)> = view.units.iter().filter(|u| u.def == borg_def).map(|u| (to_bevy(u.pos), u.speed.to_f32())).collect();
        let cine = w.resource::<CineractiveState>().on;
        let fade = w.resource::<FadeState>().alpha;
        let mut line = String::new();
        let _ = write!(line, "t={:7.2}s tick={tick} cine={} locked={o} fade={fade:.2} eye=({:.0},{:.0},{:.0}) focus=({:.0},{:.0},{:.0})", secs(tick), u8::from(cine), eye.x, eye.y, eye.z, focus.x, focus.y, focus.z);
        if let Some((p, s)) = premo {
            let _ = write!(line, " premo=({:.0},{:.0},{:.0}) v={s:.0}", p.x, p.y, p.z);
        }
        if let Some((h, hm, sh, sm)) = hull {
            let _ = write!(line, " premo_hull={h:.0}/{hm:.0} shield={sh:.0}/{sm:.0}");
        }
        for (i, (p, s)) in borg.iter().enumerate() {
            let _ = write!(line, " borg{i}=({:.0},{:.0},{:.0}) v={s:.0}", p.x, p.y, p.z);
        }
        let _ = write!(line, " | {}", lua_state(&app, &vars));
        let log = app.world().resource::<PresentationLog>().0.clone();
        for (t, p) in &log[last_log.min(log.len())..] {
            let _ = writeln!(out, "  presentation @tick {t} ({:.2}s): {p:?}", secs(*t));
        }
        last_log = log.len();
        let _ = writeln!(out, "{line}");
        samples.push(Sample { tick, cine, locked: o, fade, premo, borg, line });
        if secs(tick.saturating_sub(start)) >= seconds || state(&app) != AppState::InGame {
            break;
        }
    }
    let log = app.world().resource::<PresentationLog>().0.clone();
    if let Ok(path) = std::env::var("STA64_TRACE_OUT") {
        let _ = std::fs::write(path, out);
    }
    Some(Scene { samples, log })
}

/// The scene's order, read off the sim and the presentation log: the cut to the wide shot comes
/// when the ships are out of the gate and past it, while they still fly (the script waits for
/// "not in a queue", not for "stopped"); the clip is stopped no earlier than its own length;
/// control comes back and stays clear of fades for a while.
#[test]
fn install_gated_a1_fed1_premonition_scene_cuts_to_the_wide_shot_while_the_ships_fly() {
    let Some(scene) = drive_scene(150.0) else { return };
    let at = |pick: &dyn Fn(&Presentation) -> bool| scene.log.iter().find(|(t, p)| *t > 1900 && pick(p)).map(|(t, _)| *t);
    let shot = |name: &'static str| move |p: &Presentation| matches!(p, Presentation::Call { name: n, args } if n == "A1.CineractiveShot" && matches!(args.first(), Some(sta_sim::present::Arg::Text(t)) if t == name));
    let start = scene.samples.iter().find(|s| s.cine && s.tick > 1900).map(|s| s.tick).expect("the scene starts");
    let wide = at(&shot("PREMOSHOT")).expect("the wide shot is raised");
    // The Borg ships leave the gate about six seconds after the Premonition; the cut follows them past the gate, not their arrival.
    assert!(secs(wide - start) < 30.0, "the wide shot comes {:.1} s into the scene", secs(wide - start));
    let flying = scene.samples.iter().filter(|s| s.tick >= wide).take(3).any(|s| s.premo.iter().chain(s.borg.iter()).any(|(_, v)| *v > 1.0));
    assert!(flying, "at the cut the ships are still under way");
    // The clip: asked for with the bridge, stopped when its timer ends, not before its own length.
    let movie = scene.log.iter().find(|(_, p)| matches!(p, Presentation::Movie { in_panel: true, .. })).map(|(t, _)| *t).expect("the comm clip is asked for");
    let stop = scene.log.iter().find(|(t, p)| *t > movie && matches!(p, Presentation::StopMovie)).map(|(t, _)| *t).expect("the clip is stopped");
    let clip = clip_seconds().expect("the clip is in the install");
    assert!(secs(stop - movie) >= clip, "the clip is stopped after {:.1} s, its length is {clip:.1} s", secs(stop - movie));
    // Control: the cineractive ends, and no fade darkens the game for the next stretch.
    let off = scene.log.iter().find(|(t, p)| *t > stop.saturating_sub(2) && matches!(p, Presentation::Cineractive { on: false, .. })).map(|(t, _)| *t).expect("control returns");
    let after: Vec<&Sample> = scene.samples.iter().filter(|s| s.tick > off + 2 && secs(s.tick) < secs(off) + 30.0).collect();
    assert!(!after.is_empty() && after.iter().all(|s| !s.cine && !s.locked && s.fade == 0.0), "control returned and stays: {:?}", after.iter().find(|s| s.cine || s.locked || s.fade != 0.0).map(|s| &s.line));
    assert!(scene.samples.iter().filter(|s| s.cine && s.tick < off).all(|s| s.locked), "the camera is the script's for the whole scene");
}

fn clip_seconds() -> Option<f64> {
    let root = std::env::var("STA64_TEST_A1_INSTALL").ok()?;
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), None).ok()?;
    let opened = crate::media::locate::open(&vfs, "F1DEM1.bik").ok()?;
    opened.movie.info().duration.map(|d| d.as_secs_f64())
}
