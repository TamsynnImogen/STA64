//! Explosion definitions (#1153, plan step R2): what the explosion ODFs a unit names through
//! `fireball` and `selfDestruct` say, and the chunk settings of `ART_CFG.h`, as typed data on
//! the content database. Nothing draws them yet.
//!
//! Presentation only: not part of the content digest, because nothing in the sim reads this
//! table (ADR 0004 D10; the self-destruct blast damage keys are in the digest already, through
//! [`crate::SelfDestructBlast`]). Any explosion key the sim later reads enters the digest then
//! (ADR 0004 D10), with goldens regenerated and the `full-ci` label.
//!
//! A key that is absent stays `None`. Resolving the defaults for absent keys, and the default
//! explosion for a unit that names none, is not implemented yet (they are known at grade "read
//! from the executable, static"; a follow-up applies them with their grade).
//!
//! Evidence grades in the doc comments: "data" is a key the ODF class defines, "static" is read
//! from the executable, static; "inferred" and "not measured" are as stated. Profile rules
//! (ADR 0004 D4/D5) are those of every ODF consumer:
//! [`crate::odf::resolve_for_profile`], so includes and the first-assignment rule of the mounted
//! profile apply.

use std::collections::BTreeMap;

use sta_assets::cfg_h::CfgH;
use sta_core::Fixed;

use crate::odf::{resolve_for_profile, stem, words, Loader, Reader};
use crate::{Profile, Warning};

/// The explosion ODF a unit names: class `fireballexplode` and the other explosion classes.
///
/// The keys are read from every class; which classes use which key is established only for
/// `fireballexplode` (static). For the other classes (a rumble or omega explosion) only the
/// class label and `explSound` are meaningful until they are traced (not measured).
#[derive(Debug, Clone, PartialEq)]
pub struct ExplosionDef {
    /// The ODF's name, lower case, no folder, no extension.
    pub name: String,
    /// `classLabel`, lower case; empty when the ODF has none.
    pub class: String,
    /// `explSound`: the death sound (data; used by the sound lookup already).
    pub sound: Option<String>,
    /// `dbName`: the model of the sprite cluster the fireball plays (data; its use: static).
    pub model: Option<String>,
    /// `length`: seconds (data). Its effect on what is drawn is not established (not measured).
    pub length: Option<Fixed>,
    /// `shockwave`: the shockwave ODF started after the delay (data; static). `null` is kept
    /// as written; [`ExplosionTable::shockwave`] resolves a real name.
    pub shockwave: Option<String>,
    /// `shockwaveDelay`: seconds after the start before the shockwave is made (static).
    pub shockwave_delay: Option<Fixed>,
    /// `shockwaveMinimum`: the size the craft's radius must exceed for a shockwave (static).
    pub shockwave_minimum: Option<Fixed>,
}

/// A shockwave ODF (class `shockwavexplode`) an explosion names through `shockwave`.
#[derive(Debug, Clone, PartialEq)]
pub struct ShockwaveDef {
    /// The ODF's name, lower case, no folder, no extension.
    pub name: String,
    /// `classLabel`, lower case; empty when the ODF has none.
    pub class: String,
    /// `rays`: the number of rays of the ring (data; static).
    pub rays: Option<u32>,
    /// `color`: three components (data). Which slot of the ring it fills and whether scene
    /// light touches it is not measured; the gain applied to it is not established.
    pub color: Option<[Fixed; 3]>,
    /// `texture`: the ring's texture name (data; static).
    pub texture: Option<String>,
    /// `radiusFactor`: the ring's end size as a multiple of the craft's radius (static).
    pub radius_factor: Option<Fixed>,
    /// `fadeFactor`: the share of the end size from which the ring fades (static). The fade
    /// shape at the end is unresolved (not measured).
    pub fade_factor: Option<Fixed>,
    /// `rate`: growth in units per second (static; the outer edge measured once).
    pub rate: Option<Fixed>,
}

/// The chunk settings of `ART_CFG.h` (the pieces of a hull flying off a removed craft). A key
/// the file does not set stays `None`; applying the engine's default for it is not implemented
/// yet (the defaults are known at grade "read from the executable, static"). Every key is read
/// when present; the meaning of the values is static, except `density` and `max_lifetime`,
/// whose rule is not checked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChunkCfg {
    /// `ST3D_CHUNK_MANAGER_MAX_CHUNKS`: the cap on live chunks.
    pub max_chunks: Option<u32>,
    /// `ST3D_CHUNK_SIZE_FACTOR`.
    pub size_factor: Option<Fixed>,
    /// `ST3D_CHUNK_LIFETIME_AVG`: seconds.
    pub lifetime_average: Option<Fixed>,
    /// `ST3D_CHUNK_LIFETIME_DEVIATION`: seconds.
    pub lifetime_deviation: Option<Fixed>,
    /// `ST3D_CHUNK_MAX_ANGULAR_VELOCITY`: radians per second.
    pub max_angular_velocity: Option<Fixed>,
    /// `ST3D_CHUNK_MAX_VELOCITY`: units per second.
    pub max_velocity: Option<Fixed>,
    /// `ST3D_CHUNK_DENSITY`: read when present; its rule is not checked (not measured).
    pub density: Option<Fixed>,
    /// `ST3D_CHUNK_MAX_LIFETIME`: seconds, read when present; its rule is not checked (not measured).
    pub max_lifetime: Option<Fixed>,
}

impl ChunkCfg {
    /// The chunk keys of one `ART_CFG.h` (self-includes already expanded). A value that is
    /// not a number warns and stays unset.
    pub fn from_cfg(file: &str, cfg: &CfgH, warnings: &mut Vec<Warning>) -> ChunkCfg {
        let mut num = |key: &str| -> Option<f64> {
            let raw = cfg.raw(key)?;
            match cfg.f32(key) {
                Some(v) if v.is_finite() => Some(f64::from(v).clamp(0.0, 1.0e9)),
                _ => {
                    warnings.push(Warning::new(file, format!("{key} = {raw} is not a number; ignored")));
                    None
                }
            }
        };
        let mut fixed = |key: &str| num(key).map(Fixed::from_f64);
        let density = fixed("ST3D_CHUNK_DENSITY");
        let max_lifetime = fixed("ST3D_CHUNK_MAX_LIFETIME");
        let size_factor = fixed("ST3D_CHUNK_SIZE_FACTOR");
        let lifetime_average = fixed("ST3D_CHUNK_LIFETIME_AVG");
        let lifetime_deviation = fixed("ST3D_CHUNK_LIFETIME_DEVIATION");
        let max_angular_velocity = fixed("ST3D_CHUNK_MAX_ANGULAR_VELOCITY");
        let max_velocity = fixed("ST3D_CHUNK_MAX_VELOCITY");
        let max_chunks = num("ST3D_CHUNK_MANAGER_MAX_CHUNKS").map(|v| v.floor() as u32);
        ChunkCfg { max_chunks, size_factor, lifetime_average, lifetime_deviation, max_angular_velocity, max_velocity, density, max_lifetime }
    }
}

/// Every explosion and shockwave definition the mounted units name, and the chunk settings.
/// Keyed by lower-case ODF name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExplosionTable {
    explosions: BTreeMap<String, ExplosionDef>,
    shockwaves: BTreeMap<String, ShockwaveDef>,
    chunks: ChunkCfg,
}

impl ExplosionTable {
    /// The explosion definition of an ODF name (any case, with or without `.odf`).
    pub fn explosion(&self, name: &str) -> Option<&ExplosionDef> {
        self.explosions.get(&stem(name))
    }

    /// The shockwave definition of an ODF name.
    pub fn shockwave(&self, name: &str) -> Option<&ShockwaveDef> {
        self.shockwaves.get(&stem(name))
    }

    /// Every explosion definition, in name order.
    pub fn explosions(&self) -> impl Iterator<Item = &ExplosionDef> {
        self.explosions.values()
    }

    /// Every shockwave definition, in name order.
    pub fn shockwaves(&self) -> impl Iterator<Item = &ShockwaveDef> {
        self.shockwaves.values()
    }

    /// The `ART_CFG.h` chunk settings.
    pub fn chunks(&self) -> &ChunkCfg {
        &self.chunks
    }

    pub fn is_empty(&self) -> bool {
        self.explosions.is_empty() && self.shockwaves.is_empty()
    }

    pub(crate) fn set_chunks(&mut self, chunks: ChunkCfg) {
        self.chunks = chunks;
    }

    /// Read the definitions of `names` (a unit's `fireball` or `selfDestruct`) and the
    /// shockwaves they name. A name that does not resolve is skipped here: the unit loader
    /// already warns for it. A shockwave that does not resolve warns.
    pub(crate) fn read<'a>(
        names: impl IntoIterator<Item = &'a str>,
        profile: Profile,
        loader: &mut Loader<'_>,
        warnings: &mut Vec<Warning>,
    ) -> ExplosionTable {
        let mut table = ExplosionTable::default();
        for name in names {
            let key = stem(name);
            if key.is_empty() || key == "null" || table.explosions.contains_key(&key) {
                continue;
            }
            let Ok(odf) = resolve_for_profile(&key, profile, loader) else { continue };
            let file = format!("{key}.odf");
            let mut r = Reader { odf: &odf, file: &file, warnings };
            let def = ExplosionDef {
                class: odf.class_label().unwrap_or("").trim().to_ascii_lowercase(),
                sound: r.text("explSound"),
                model: r.text("dbName"),
                length: nonneg(&mut r, "length"),
                shockwave: r.text("shockwave"),
                shockwave_delay: nonneg(&mut r, "shockwaveDelay"),
                shockwave_minimum: nonneg(&mut r, "shockwaveMinimum"),
                name: key.clone(),
            };
            let wave = def.shockwave.as_deref().map(stem).filter(|w| !w.is_empty() && w != "null");
            table.explosions.insert(key.clone(), def);
            let Some(wave) = wave else { continue };
            if table.shockwaves.contains_key(&wave) {
                continue;
            }
            match resolve_for_profile(&wave, profile, loader) {
                Ok(odf) => {
                    let file = format!("{wave}.odf");
                    let mut r = Reader { odf: &odf, file: &file, warnings };
                    let def = ShockwaveDef {
                        class: odf.class_label().unwrap_or("").trim().to_ascii_lowercase(),
                        rays: r.num("rays").map(|v| v.max(0.0).floor().min(1.0e6) as u32),
                        color: color(&mut r),
                        texture: r.text("texture"),
                        radius_factor: nonneg(&mut r, "radiusFactor"),
                        fade_factor: nonneg(&mut r, "fadeFactor"),
                        rate: nonneg(&mut r, "rate"),
                        name: wave.clone(),
                    };
                    table.shockwaves.insert(wave, def);
                }
                Err(e) => warnings.push(Warning::new(format!("{key}.odf"), format!("Shockwave ODF unavailable: {wave}.odf (shockwave). {e}"))),
            }
        }
        table
    }
}

/// A non-negative number, `None` when the key is absent or not a number (the latter warns).
fn nonneg(r: &mut Reader<'_>, key: &str) -> Option<Fixed> {
    r.num(key).map(|v| Fixed::from_f64(v.clamp(0.0, 1.0e9)))
}

/// `color`: three numbers separated by white space. Anything else warns and stays unset.
fn color(r: &mut Reader<'_>) -> Option<[Fixed; 3]> {
    let odf = r.odf;
    let raw = odf.str("color")?;
    let parts: Vec<Option<f64>> = words(raw).iter().map(|w| w.parse::<f64>().ok().filter(|v| v.is_finite())).collect();
    match parts.as_slice() {
        [Some(a), Some(b), Some(c)] => Some([a, b, c].map(|v| Fixed::from_f64(v.clamp(0.0, 1.0e9)))),
        _ => {
            r.warnings.push(Warning::new(r.file, format!("color = {raw} is not three numbers; ignored")));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn table(files: &[(&str, &str)], names: &[&str]) -> (ExplosionTable, Vec<Warning>) {
        let map: HashMap<String, String> = files.iter().map(|(n, t)| (n.to_ascii_lowercase(), (*t).to_owned())).collect();
        let mut loader = move |n: &str, v: usize| -> Option<String> {
            if v > 0 {
                return None;
            }
            let k = n.to_ascii_lowercase();
            map.get(k.strip_suffix(".odf").unwrap_or(&k)).cloned()
        };
        let mut warnings = Vec::new();
        let t = ExplosionTable::read(names.iter().copied(), Profile::Sta2Classic, &mut loader, &mut warnings);
        (t, warnings)
    }

    fn fx(v: f64) -> Fixed {
        Fixed::from_f64(v)
    }

    #[test]
    fn explosion_and_shockwave_keys_are_read_through_includes() {
        let (t, w) = table(
            &[
                ("base", "classLabel = \"fireballexplode\"\nexplSound = \"boom.wav\"\ndbName = \"cluster\"\nlength = 2.5\n"),
                ("boom", "#include \"base.odf\"\nshockwave = \"ring\"\nshockwaveDelay = 0.5\nshockwaveMinimum = 30.0\n"),
                ("ring", "classLabel = \"shockwavexplode\"\nrate = 100\ntexture = \"tex\"\ncolor =\t1.0\t0.5\t0.0\nradiusFactor = 8.0\nfadeFactor = 0.25\nrays = 12\n"),
            ],
            &["Boom.odf"],
        );
        assert!(w.is_empty(), "{w:?}");
        let e = t.explosion("boom").unwrap();
        assert_eq!(e.class, "fireballexplode");
        assert_eq!((e.sound.as_deref(), e.model.as_deref(), e.length), (Some("boom.wav"), Some("cluster"), Some(fx(2.5))));
        assert_eq!((e.shockwave.as_deref(), e.shockwave_delay, e.shockwave_minimum), (Some("ring"), Some(fx(0.5)), Some(fx(30.0))));
        let s = t.shockwave("RING.odf").unwrap();
        assert_eq!((s.class.as_str(), s.rays, s.texture.as_deref()), ("shockwavexplode", Some(12), Some("tex")));
        assert_eq!(s.color, Some([fx(1.0), fx(0.5), fx(0.0)]));
        assert_eq!((s.radius_factor, s.fade_factor, s.rate), (Some(fx(8.0)), Some(fx(0.25)), Some(fx(100.0))));
        assert_eq!(t.explosions().count(), 1);
        assert_eq!(t.shockwaves().count(), 1);
    }

    #[test]
    fn the_first_assignment_of_a_key_wins_under_the_classic_profile() {
        let (t, _) = table(&[("boom", "classLabel = \"fireballexplode\"\nlength = 1.5\nlength = 9.0\n")], &["boom"]);
        assert_eq!(t.explosion("boom").unwrap().length, Some(fx(1.5)));
    }

    #[test]
    fn absent_keys_stay_unset_and_null_or_missing_names_are_skipped() {
        let (t, w) = table(&[("plain", "classLabel = \"fireballexplode\"\nshockwave = \"null\"\n")], &["plain", "null", "", "missing"]);
        let e = t.explosion("plain").unwrap();
        assert_eq!((e.sound.as_ref(), e.model.as_ref(), e.length, e.shockwave_delay, e.shockwave_minimum), (None, None, None, None, None));
        assert_eq!(t.shockwaves().count(), 0);
        assert!(t.explosion("missing").is_none() && w.is_empty(), "{w:?}");
        assert_eq!(t.explosions().count(), 1);
    }

    #[test]
    fn malformed_values_warn_and_stay_unset() {
        let (t, w) = table(
            &[
                ("boom", "classLabel = \"fireballexplode\"\nlength = long\nshockwave = \"ring\"\nshockwaveDelay = -2\n"),
                ("ring", "classLabel = \"shockwavexplode\"\ncolor = 1.0 red 0.0\nrays = many\nrate = 1e30\n"),
            ],
            &["boom"],
        );
        let e = t.explosion("boom").unwrap();
        assert_eq!((e.length, e.shockwave_delay), (None, Some(Fixed::ZERO)));
        let s = t.shockwave("ring").unwrap();
        assert_eq!((s.color, s.rays), (None, None));
        assert_eq!(s.rate, Some(fx(1.0e9)), "a huge value is clamped to the same ceiling as colour and chunk values");
        let text = format!("{w:?}");
        assert!(text.contains("length") && text.contains("color") && text.contains("rays"), "{text}");
    }

    #[test]
    fn a_shockwave_that_does_not_resolve_warns() {
        let (t, w) = table(&[("boom", "classLabel = \"fireballexplode\"\nshockwave = \"gone\"\n")], &["boom"]);
        assert!(t.shockwave("gone").is_none());
        assert!(w.iter().any(|w| w.file == "boom.odf" && w.message.contains("gone.odf")), "{w:?}");
    }

    #[test]
    fn chunk_keys_are_read_and_unset_keys_stay_none() {
        let cfg = CfgH::parse("int ST3D_CHUNK_MANAGER_MAX_CHUNKS = 100;\nfloat ST3D_CHUNK_SIZE_FACTOR = 0.25;\nfloat ST3D_CHUNK_LIFETIME_AVG = 2.0;\nfloat ST3D_CHUNK_MAX_VELOCITY = 50.0;\nfloat ST3D_CHUNK_DENSITY = 0.5;\n");
        let mut w = Vec::new();
        let c = ChunkCfg::from_cfg("ART_CFG.h", &cfg, &mut w);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((c.max_chunks, c.size_factor, c.lifetime_average, c.max_velocity, c.density), (Some(100), Some(fx(0.25)), Some(fx(2.0)), Some(fx(50.0)), Some(fx(0.5))));
        assert_eq!((c.lifetime_deviation, c.max_angular_velocity, c.max_lifetime), (None, None, None));
        assert_eq!(ChunkCfg::from_cfg("ART_CFG.h", &CfgH::parse(""), &mut Vec::new()), ChunkCfg::default());
    }

    fn loaded(length: &str) -> crate::ContentDb {
        let files = [
            ("ship", "classLabel = \"craft\"\nunitName = \"Ship\"\nfireball = \"boom\"\nselfDestruct = \"selfboom\"\n".to_owned()),
            ("bare", "classLabel = \"craft\"\nunitName = \"Bare\"\n".to_owned()),
            ("boom", format!("classLabel = \"fireballexplode\"\nlength = {length}\nshockwave = \"ring\"\n")),
            ("selfboom", "classLabel = \"fireballexplode\"\n".to_owned()),
            ("ring", "classLabel = \"shockwavexplode\"\nrays = 4\n".to_owned()),
        ];
        let stems: Vec<String> = ["ship", "bare"].map(String::from).to_vec();
        let (db, w) = crate::odf::load_with(Profile::Sta2Classic, &crate::Overrides::default(), &stems, &mut |name, version| {
            (version == 0).then(|| files.iter().find(|(f, _)| f.eq_ignore_ascii_case(name.trim_end_matches(".odf"))).map(|(_, t)| t.clone())).flatten()
        })
        .unwrap();
        assert!(w.is_empty(), "{w:?}");
        db
    }

    #[test]
    fn the_database_resolves_a_units_fireball_and_self_destruct_and_the_digest_ignores_them() {
        let db = loaded("2.5");
        let ship = db.unit(db.key("ship").unwrap()).unwrap();
        assert_eq!(db.explosion_of(ship).map(|e| e.name.as_str()), Some("boom"));
        assert_eq!(db.self_destruct_explosion_of(ship).map(|e| e.name.as_str()), Some("selfboom"));
        assert_eq!(db.explosions().shockwave("ring").unwrap().rays, Some(4));
        let bare = db.unit(db.key("bare").unwrap()).unwrap();
        assert!(db.explosion_of(bare).is_none() && db.self_destruct_explosion_of(bare).is_none());
        // Presentation only (ADR 0004 D10): another length does not change the content digest.
        assert_eq!(db.digest(), loaded("9.5").digest());
        assert_eq!(loaded("2.5").explosions().explosion("boom").unwrap().length, Some(fx(2.5)));
        assert_ne!(loaded("9.5").explosions().explosion("boom").unwrap().length, Some(fx(2.5)));
    }

    #[test]
    fn a_malformed_chunk_value_warns() {
        let cfg = CfgH::parse("float ST3D_CHUNK_SIZE_FACTOR = big;\n");
        let mut w = Vec::new();
        let c = ChunkCfg::from_cfg("ART_CFG.h", &cfg, &mut w);
        assert_eq!(c.size_factor, None);
        assert!(w.iter().any(|w| w.message.contains("ST3D_CHUNK_SIZE_FACTOR")), "{w:?}");
    }
}
