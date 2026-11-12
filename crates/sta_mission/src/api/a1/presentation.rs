//! Armada I presentation calls: bridge scenes and movies, cineractives,
//! camera shots, fades, titles, audio messages, the minimap and the
//! objectives display. Each queues `Edit::Present` from `PlayerId::SCRIPT`;
//! nothing here plays media or moves a camera.
//!
//! # Bridge scenes (measured on the Armada I install)
//!
//! `BridgeDisplay::BridgeType` is an integer in the DLLs: 0, 1, 2, 3, 4. The
//! values correlate with the race of the bridge movies played in the same
//! script (`F*.bik` Federation, `K*.bik` Klingon, `R*.bik` Romulan, `B*.bik`
//! Borg): 1 Federation, 2 Klingon, 3 Romulan, 4 Borg. 0 appears in finale and
//! Klingon 4/5 scripts with no race of its own; its meaning is not measured,
//! so it is passed through as 0 with an empty race name.
//!
//! The scene calls are `Presentation::Call` `A1.PreLoadBridgeScene` /
//! `A1.SetBridgeSceneRace` with `[Int type, Text race]` and
//! `A1.ShowBridgeScene` / `A1.ShowBridgeOnScreen` with `[Bool]`, for the
//! bridge display (MED-03 and UI) to consume. Movies use the typed events:
//! `PlayBridgeMovie(name, flag)` is `Presentation::Movie { in_panel: true,
//! skippable: true }`, `StopMovie` is `Presentation::StopMovie`. The flag (1
//! in most stock calls, 0 and -1 for the `Martok.bik` and `Sela.bik`
//! character movies) is validated and its meaning is not measured, so it is
//! dropped.
//!
//! # Completion
//!
//! `IsMoviePlaying` and `IsAudioMessageDone` need the client's answer. Until
//! a host reports it with `A1.MovieDone()` / `A1.AudioMessageDone(id)` (not
//! DLL calls; MED-03 / the audio mixer use them) a movie or message counts as
//! still playing until stopped.
//!
//! # Titles (measured)
//!
//! `ShowTitle` / `ShowSubtitle(x, y, width, height, text, flag)` pass the
//! rectangle in 640x480 screen pixels: titles `(0, 0, 200, 70)`, subtitles
//! `(0, 380, 640, 40)` or `(65, 140, 640, 40)`; the flag is 1 in every stock
//! call and not measured. The typed `Presentation::Title` (no placement) is
//! queued and the rectangle is kept in `A1.ShowTitle` / `A1.ShowSubtitle`
//! calls `[Int id, x, y, width, height, Bool flag]`.

use mlua::{Function, Lua, Table, Value};
use sta_sim::present::{Arg, Presentation};

use super::{A, Env, args_of, def, objects, rename_error};

const RACES: [&str; 5] = ["", "federation", "klingon", "romulan", "borg"];

fn bridge_type(name: &str, a: &A) -> mlua::Result<(i64, String)> {
    let n = a.int();
    let race = usize::try_from(n)
        .ok()
        .and_then(|i| RACES.get(i))
        .ok_or_else(|| mlua::Error::runtime(format!("A1.{name} requires a bridge type 0..=4")))?;
    Ok((n, (*race).to_owned()))
}

fn bridge_race(lua: &Lua, env: &Env, name: &'static str) -> mlua::Result<Function> {
    def(lua, env, name, &["i"], move |_, e, _, a| {
        let (n, race) = bridge_type(name, &a[0])?;
        let mut args = vec![Arg::Int(n)];
        if !race.is_empty() {
            args.push(Arg::Text(race));
        }
        e.call(name, args)?;
        Ok(Value::Nil)
    })
}

fn flag_call(lua: &Lua, env: &Env, name: &'static str) -> mlua::Result<Function> {
    def(lua, env, name, &["b"], move |_, e, _, a| {
        e.call(name, args_of(a))?;
        Ok(Value::Nil)
    })
}

fn cineractive(lua: &Lua, on: bool) -> mlua::Result<()> {
    // The same singleton the converted `Cineractive.active = on` drives.
    lua.load(format!("Cineractive.active = {on}")).exec()
}

fn fade(lua: &Lua, env: &Env, name: &'static str, out: bool) -> mlua::Result<Function> {
    // `FadeIn/FadeOut(f, f, f)`: the stock values are (1,0,0), (3,0,0),
    // (4,5,0), (5,7,0)... The first is read as the fade time in seconds (as
    // MMM's fadeIn/fadeOut); the other two are retained, meaning not measured.
    def(lua, env, name, &["fff"], move |_, e, _, a| {
        e.call(name, args_of(a))?;
        let seconds = a[0].num();
        if seconds >= 0.0 {
            e.present(name, Presentation::Fade { out, seconds: sta_core::Fixed::from_f64(seconds), rgb: [0, 0, 0] })?;
        }
        Ok(Value::Nil)
    })
}

fn title(lua: &Lua, env: &Env, name: &'static str, subtitle: bool) -> mlua::Result<Function> {
    def(lua, env, name, &["iiiisb"], move |_, e, _, a| {
        let id = {
            let mut st = e.st.borrow_mut();
            st.next_title = st
                .next_title
                .checked_add(1)
                .ok_or_else(|| mlua::Error::runtime(format!("A1.{name}: title ids exhausted")))?;
            st.next_title
        };
        e.present(name, Presentation::Title { id, text: a[4].text().to_owned(), subtitle, seconds: None })?;
        e.call(
            name,
            vec![Arg::Int(i64::from(id)), Arg::Int(a[0].int()), Arg::Int(a[1].int()), Arg::Int(a[2].int()), Arg::Int(a[3].int()), Arg::Bool(a[5].boolean())],
        )?;
        Ok(Value::Integer(i64::from(id)))
    })
}

fn remove_title(lua: &Lua, env: &Env, name: &'static str) -> mlua::Result<Function> {
    // `RemoveTitle(id, bool)`: the bool is not measured and ignored.
    def(lua, env, name, &["ib"], move |_, e, _, a| {
        let id = u32::try_from(a[0].int()).ok().filter(|id| *id >= 1 && *id <= e.st.borrow().next_title);
        let Some(id) = id else {
            return Err(mlua::Error::runtime(format!("A1.{name}: {} is not a title id this mission was given", a[0].int())));
        };
        e.present(name, Presentation::RemoveTitle { id })?;
        Ok(Value::Nil)
    })
}

fn plain_call(lua: &Lua, env: &Env, name: &'static str, specs: &'static [&'static str]) -> mlua::Result<Function> {
    def(lua, env, name, specs, move |_, e, _, a| {
        e.call(name, args_of(a))?;
        Ok(Value::Nil)
    })
}

pub(super) fn install(lua: &Lua, env: &Env, a2: &Table, table: &Table) -> mlua::Result<()> {
    // Bridge scenes and movies.
    table.set("PreLoadBridgeScene", bridge_race(lua, env, "PreLoadBridgeScene")?)?;
    table.set("SetBridgeSceneRace", bridge_race(lua, env, "SetBridgeSceneRace")?)?;
    table.set("ShowBridgeScene", flag_call(lua, env, "ShowBridgeScene")?)?;
    table.set("ShowBridgeOnScreen", flag_call(lua, env, "ShowBridgeOnScreen")?)?;
    table.set("PlayBridgeMovie", def(lua, env, "PlayBridgeMovie", &["si"], |_, e, _, a| {
        let name = a[0].text();
        if name.is_empty() {
            return Err(mlua::Error::runtime("A1.PlayBridgeMovie requires a movie name"));
        }
        e.present("PlayBridgeMovie", Presentation::Movie { name: name.to_owned(), in_panel: true, skippable: true })?;
        e.st.borrow_mut().movie_playing = true;
        Ok(Value::Nil)
    })?)?;
    table.set("StopMovie", def(lua, env, "StopMovie", &[""], |_, e, _, _| {
        e.present("StopMovie", Presentation::StopMovie)?;
        e.st.borrow_mut().movie_playing = false;
        Ok(Value::Nil)
    })?)?;
    table.set("IsMoviePlaying", def(lua, env, "IsMoviePlaying", &[""], |_, e, _, _| {
        Ok(Value::Boolean(e.st.borrow().movie_playing))
    })?)?;
    // Host-side completion reports (not DLL calls).
    table.set("MovieDone", def(lua, env, "MovieDone", &[""], |_, e, _, _| {
        e.st.borrow_mut().movie_playing = false;
        Ok(Value::Nil)
    })?)?;
    table.set("AudioMessageDone", def(lua, env, "AudioMessageDone", &["i"], |_, e, _, a| {
        e.st.borrow_mut().sounds_active.remove(&(a[0].int() as u32));
        Ok(Value::Nil)
    })?)?;

    // Cineractives and camera.
    table.set("CineractiveStart", def(lua, env, "CineractiveStart", &[""], |lua, _, _, _| {
        cineractive(lua, true)?;
        Ok(Value::Nil)
    })?)?;
    table.set("CineractiveEnd", def(lua, env, "CineractiveEnd", &[""], |lua, _, _, _| {
        cineractive(lua, false)?;
        Ok(Value::Nil)
    })?)?;
    table.set("FadeIn", fade(lua, env, "FadeIn", false)?)?;
    table.set("FadeOut", fade(lua, env, "FadeOut", true)?)?;
    table.set("CineractiveCameraOffset", def(lua, env, "CineractiveCameraOffset", &["sfb", "fffb"], |_, e, _, a| {
        e.call("CineractiveCameraOffset", args_of(a))?;
        Ok(Value::Nil)
    })?)?;
    // Seven overloads: `(from, at)` handles; `(path, point-or-time, handle)`;
    // `(path, point-or-time, path, point-or-time)`. The numbers keep their Lua
    // subtype and no meaning is assigned to them (as for A2).
    table.set("CineractiveShot", def(lua, env, "CineractiveShot", &["hh", "snh", "snsn"], |_, e, _, a| {
        for value in a {
            if let A::Handle(id) = value {
                e.unit("CineractiveShot", *id)?;
            }
        }
        e.call("CineractiveShot", args_of(a))?;
        Ok(Value::Nil)
    })?)?;
    table.set("CenterCamera", def(lua, env, "CenterCamera", &["si"], |_, e, _, a| {
        e.call("CenterCamera", args_of(a))?;
        Ok(Value::Nil)
    })?)?;

    // Titles, audio, minimap, objectives display.
    table.set("ShowTitle", title(lua, env, "ShowTitle", false)?)?;
    table.set("ShowSubtitle", title(lua, env, "ShowSubtitle", true)?)?;
    table.set("RemoveTitle", remove_title(lua, env, "RemoveTitle")?)?;
    table.set("RemoveSubtitle", remove_title(lua, env, "RemoveSubtitle")?)?;
    table.set("AudioMessage", def(lua, env, "AudioMessage", &["s"], |_, e, _, a| {
        let id = {
            let mut st = e.st.borrow_mut();
            st.next_sound = st
                .next_sound
                .checked_add(1)
                .ok_or_else(|| mlua::Error::runtime("A1.AudioMessage: sound ids exhausted"))?;
            let id = st.next_sound;
            st.sounds_active.insert(id);
            id
        };
        e.present("AudioMessage", Presentation::Sound { id, name: a[0].text().to_owned(), at: None })?;
        Ok(Value::Integer(i64::from(id)))
    })?)?;
    table.set("StopAudioMessage", def(lua, env, "StopAudioMessage", &["i"], |_, e, _, a| {
        let id = u32::try_from(a[0].int()).map_err(|_| mlua::Error::runtime("A1.StopAudioMessage requires a sound id"))?;
        e.st.borrow_mut().sounds_active.remove(&id);
        e.present("StopAudioMessage", Presentation::StopSound { id })?;
        Ok(Value::Nil)
    })?)?;
    table.set("IsAudioMessageDone", def(lua, env, "IsAudioMessageDone", &["i"], |_, e, _, a| {
        let active = u32::try_from(a[0].int()).is_ok_and(|id| e.st.borrow().sounds_active.contains(&id));
        Ok(Value::Boolean(!active))
    })?)?;
    table.set("MinimapMessage", plain_call(lua, env, "MinimapMessage", &["iif", "sifi"])?)?;
    table.set("GridVisible", plain_call(lua, env, "GridVisible", &["b"])?)?;
    table.set("RemoveBillboardEffect", plain_call(lua, env, "RemoveBillboardEffect", &["i"])?)?;
    table.set("FadeRemove", plain_call(lua, env, "FadeRemove", &[""])?)?;
    table.set("StartCredits", plain_call(lua, env, "StartCredits", &[""])?)?;
    table.set("EndCredits", plain_call(lua, env, "EndCredits", &[""])?)?;
    table.set("Starfield_Load_Background_Geometry", plain_call(lua, env, "Starfield_Load_Background_Geometry", &["s"])?)?;
    table.set("AddTransporterEffect", def(lua, env, "AddTransporterEffect", &["h"], |_, e, _, a| {
        objects::transporter(e, a[0].id())?;
        Ok(Value::Nil)
    })?)?;

    let visibility: Function = a2.get("ObjectivesDisplay_Set_Visibility")?;
    table.set("ObjectivesDisplay_Set_Visibility", def(lua, env, "ObjectivesDisplay_Set_Visibility", &["b"], move |_, _, _, a| {
        visibility.call::<()>(a[0].boolean()).map_err(rename_error)?;
        Ok(Value::Nil)
    })?)?;
    // The A1 call takes only the file; a fresh load resets goal states.
    let load: Function = a2.get("ObjectivesDisplay_Set_Text_From_File")?;
    table.set("ObjectivesDisplay_Set_Text_From_File", def(lua, env, "ObjectivesDisplay_Set_Text_From_File", &["s"], move |_, _, _, a| {
        load.call::<()>((a[0].text().to_owned(), true)).map_err(rename_error)?;
        Ok(Value::Nil)
    })?)?;
    Ok(())
}
