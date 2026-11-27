//! The client's record in a save (ADR 0010 V2: one opaque, versioned record written and read by the
//! client). JSON, so a field can be added or dropped without a new version: unknown fields are
//! ignored, a missing or unreadable field takes its default and is reported, and only another
//! `version` or text that is not JSON gives up the whole record. A record the client cannot read
//! never makes a save unloadable: the match loads with defaults and a notice says what was left out.

use serde_json::{Value, json};
use sta_sim::command::Command;

use crate::clock::MatchSpeed;
use crate::render::fog::ExploredRecord;

/// The record layout this build writes and reads.
pub const VERSION: u32 = 1;

/// The orbit camera's pose (the capture index records the same fields).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraRec {
    pub focus: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
}

/// What the client saves for itself. Not in it, and rebuilt as on a fresh start: selection, HUD mode
/// and pages, minimap, notices, music, cursor. Control groups wait for the input claim (#960).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientRecord {
    pub camera: Option<CameraRec>,
    /// The local seat's explored memory.
    pub explored: Option<ExploredRecord>,
    /// The match's tick pacing.
    pub speed: MatchSpeed,
    /// Commands on their way to the next tick when the game was saved.
    pub in_flight: Vec<Command>,
}

impl Default for ClientRecord {
    fn default() -> Self {
        ClientRecord { camera: None, explored: None, speed: MatchSpeed::Normal, in_flight: Vec::new() }
    }
}

fn speed_name(s: MatchSpeed) -> &'static str {
    match s {
        MatchSpeed::Slow => "slow",
        MatchSpeed::Normal => "normal",
        MatchSpeed::Fast => "fast",
    }
}

impl ClientRecord {
    pub fn encode(&self) -> Vec<u8> {
        let camera = self.camera.map(|c| json!({"focus": c.focus, "yaw": c.yaw, "pitch": c.pitch, "dist": c.dist}));
        let explored = self.explored.as_ref().map(|e| {
            let runs: Vec<u64> = e.runs.iter().flat_map(|&(v, n)| [u64::from(v), u64::from(n)]).collect();
            json!({"min": e.min, "w": e.w, "h": e.h, "runs": runs})
        });
        let v = json!({
            "camera": camera,
            "explored": explored,
            "speed": speed_name(self.speed),
            "in_flight": serde_json::to_value(&self.in_flight).unwrap_or(Value::Null),
        });
        v.to_string().into_bytes()
    }

    /// The record, and what was left out of it. `Err` when the version is not this build's or the
    /// bytes are not a record at all: the caller then starts with [`ClientRecord::default`].
    pub fn decode(version: u32, bytes: &[u8]) -> Result<(ClientRecord, Vec<String>), String> {
        if version != VERSION {
            return Err(format!("the client record is version {version}, this build reads version {VERSION}"));
        }
        let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("the client record is damaged: {e}"))?;
        if !v.is_object() {
            return Err("the client record is damaged: not an object".into());
        }
        let mut notes = Vec::new();
        let mut rec = ClientRecord::default();
        match &v["camera"] {
            Value::Null => {}
            c => match camera_of(c) {
                Some(c) => rec.camera = Some(c),
                None => notes.push("the camera position".to_owned()),
            },
        }
        match &v["explored"] {
            Value::Null => {}
            e => match explored_of(e) {
                Some(e) => rec.explored = Some(e),
                None => notes.push("the explored map".to_owned()),
            },
        }
        match v["speed"].as_str() {
            Some("slow") => rec.speed = MatchSpeed::Slow,
            Some("normal") | None => {}
            Some("fast") => rec.speed = MatchSpeed::Fast,
            Some(_) => notes.push("the game speed".to_owned()),
        }
        match &v["in_flight"] {
            Value::Null => {}
            c => match serde_json::from_value(c.clone()) {
                Ok(cmds) => rec.in_flight = cmds,
                Err(_) => notes.push("the orders on their way".to_owned()),
            },
        }
        Ok((rec, notes))
    }
}

fn floats<const N: usize>(v: &Value) -> Option<[f32; N]> {
    let a = v.as_array().filter(|a| a.len() == N)?;
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64().filter(|f| f.is_finite())? as f32;
    }
    Some(out)
}

fn camera_of(v: &Value) -> Option<CameraRec> {
    let f = |k: &str| v[k].as_f64().filter(|f| f.is_finite()).map(|f| f as f32);
    Some(CameraRec { focus: floats(&v["focus"])?, yaw: f("yaw")?, pitch: f("pitch")?, dist: f("dist")? })
}

fn explored_of(v: &Value) -> Option<ExploredRecord> {
    let flat = v["runs"].as_array()?;
    if flat.len() % 2 != 0 {
        return None;
    }
    let mut runs = Vec::with_capacity(flat.len() / 2);
    for pair in flat.chunks(2) {
        runs.push((u8::try_from(pair[0].as_u64()?).ok()?, u32::try_from(pair[1].as_u64()?).ok()?));
    }
    Some(ExploredRecord { min: floats(&v["min"])?, w: usize::try_from(v["w"].as_u64()?).ok()?, h: usize::try_from(v["h"].as_u64()?).ok()?, runs })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> ClientRecord {
        ClientRecord {
            camera: Some(CameraRec { focus: [1.5, 0.0, -300.25], yaw: 0.3, pitch: -0.7, dist: 812.5 }),
            explored: Some(ExploredRecord { min: [-100.0, -50.5], w: 3, h: 2, runs: vec![(0, 2), (255, 1), (130, 3)] }),
            speed: MatchSpeed::Fast,
            in_flight: Vec::new(),
        }
    }

    #[test]
    fn a_record_round_trips_exactly() {
        let r = full();
        let (back, notes) = ClientRecord::decode(VERSION, &r.encode()).unwrap();
        assert_eq!(back, r);
        assert!(notes.is_empty());
    }

    #[test]
    fn another_version_or_garbage_gives_up_the_whole_record() {
        assert!(ClientRecord::decode(VERSION + 1, &full().encode()).unwrap_err().contains("version"));
        assert!(ClientRecord::decode(VERSION, b"\x00 not json").is_err());
        assert!(ClientRecord::decode(VERSION, b"[1]").is_err());
    }

    #[test]
    fn an_unreadable_field_is_left_out_and_named_and_the_rest_survives() {
        let mut v: Value = serde_json::from_slice(&full().encode()).unwrap();
        v["camera"] = json!({"focus": "up"});
        v["future_field"] = json!(7);
        let (rec, notes) = ClientRecord::decode(VERSION, v.to_string().as_bytes()).unwrap();
        assert_eq!(rec.camera, None);
        assert_eq!(rec.explored, full().explored);
        assert_eq!(rec.speed, MatchSpeed::Fast);
        assert_eq!(notes, ["the camera position"]);
    }

    #[test]
    fn a_missing_field_is_the_default_without_a_note() {
        let (rec, notes) = ClientRecord::decode(VERSION, b"{}").unwrap();
        assert_eq!(rec, ClientRecord::default());
        assert!(notes.is_empty());
    }
}
