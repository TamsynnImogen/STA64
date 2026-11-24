//! An editable map document. Indices stay stable while objects are removed so
//! selection and undo records can refer to the same placement.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::prelude::Resource;
use sta_bzn::{Map, MapObject};
use sta_sim::{Fixed, source::ObjectProperties};

#[derive(Resource, Clone)]
pub struct Document {
    pub path: String,
    pub map: Map,
    pub objects: Vec<Option<MapObject>>,
    source: Arc<[u8]>,
    next_entity_id: Option<i32>,
    properties: BTreeMap<i32, ObjectProperties>,
}

impl Document {
    pub fn new(path: String, bytes: Vec<u8>, map: Map) -> Result<Self, String> {
        if path.to_ascii_lowercase().ends_with(".sol") {
            let original = sta_bzn::sol::parse(&bytes).map_err(|error| error.to_string())?;
            if original.to_map() != map {
                return Err("SOL map does not match its source bytes".into());
            }
        }
        let mut properties: BTreeMap<_, _> = sta_bzn::object_editor_ext::read(&bytes)
            .into_iter()
            .map(|(id, values)| (id, ObjectProperties {
                captain: values.captain,
                registry: values.registry,
                shields: values.shields.map(|faces| faces.map(|pair| pair.map(|value| Fixed::from_f64(f64::from(value))))),
                ..Default::default()
            }))
            .collect();
        if path.starts_with("saved:") {
            let file = super::saved_map_file(&path)?;
            if let Some(saved) = read_properties(&file, &bytes)? {
                properties = saved;
            }
        }
        let objects = map.objects.iter().cloned().map(|mut object| {
            if let Some(name) = properties.get(&object.entity_id).and_then(|p| p.instance_name.as_ref()) {
                object.instance_name = name.clone();
            }
            Some(object)
        }).collect();
        let next_entity_id = map
            .objects
            .iter()
            .map(|object| object.entity_id)
            .max()
            .unwrap_or(-1)
            .checked_add(1);
        Ok(Self {
            path,
            map,
            objects,
            source: bytes.into(),
            next_entity_id,
            properties,
        })
    }

    pub fn object(&self, index: usize) -> Option<&MapObject> {
        self.objects.get(index)?.as_ref()
    }

    pub(super) fn properties(&self, id: i32) -> Option<&ObjectProperties> {
        self.properties.get(&id)
    }

    pub(super) fn update_object(&mut self, index: usize, object: MapObject, properties: ObjectProperties) {
        if properties == ObjectProperties::default() {
            self.properties.remove(&object.entity_id);
        } else {
            self.properties.insert(object.entity_id, properties);
        }
        self.objects[index] = Some(object);
    }

    pub(crate) fn restore_properties(
        &self,
        sim: &mut sta_sim::Sim,
        ids: &BTreeMap<sta_sim::EntityId, usize>,
        names: &mut crate::ui::hud::instance_names::InstanceNames,
        directional: bool,
    ) -> Result<(), String> {
        use sta_sim::{WorldSource, source::PlacementEdit};

        let view = sim.view();
        for (&id, &index) in ids {
            let (Some(object), Some(unit)) = (self.object(index), view.unit(id)) else { continue };
            let authored = self.properties(object.entity_id);
            names.set_overrides(id, Some(&object.instance_name), authored.and_then(|p| p.captain.as_deref()),
                authored.and_then(|p| p.registry.as_deref()));
            let Some(mut properties) = authored.cloned() else { continue };
            // The raw A2FO record survives in the document. The simulation
            // can currently restore only a common maximum across facings.
            if properties.shields.is_some_and(|faces| faces.iter().any(|face| {
                face[1] != faces[0][1] || (!directional && *face != faces[0])
            })) {
                properties.shields = None;
            }
            let edit = PlacementEdit::Inspect {
                id,
                owner: unit.owner,
                at: unit.pos,
                heading: unit.heading,
                label: None,
                name: properties.instance_name.clone(),
                properties,
            };
            if sim.edit_placement(edit).is_none() {
                return Err(format!("Cannot restore inspector values for object {} ({})", object.entity_id, object.odf));
            }
        }
        Ok(())
    }

    pub fn can_change_objects(&self) -> bool {
        self.map.a1.is_none() && self.map.version != sta_bzn::sol::VERSION
    }

    pub fn move_to(&mut self, index: usize, position: [f32; 3]) -> bool {
        if !position.iter().all(|value| value.is_finite()) {
            return false;
        }
        let Some(object) = self.objects.get_mut(index).and_then(Option::as_mut) else {
            return false;
        };
        object.transform[9..12].copy_from_slice(&position);
        if self.map.a1.is_none() {
            object.initial_position = Some(position);
        }
        true
    }

    pub fn rotate(&mut self, index: usize, quarter_turns: i32) -> bool {
        let Some(object) = self.objects.get_mut(index).and_then(Option::as_mut) else {
            return false;
        };
        let turns = quarter_turns.rem_euclid(4);
        for _ in 0..turns {
            // Matrix34's first and third basis rows are right and front.
            for axis in 0..3 {
                let right = object.transform[axis];
                object.transform[axis] = -object.transform[6 + axis];
                object.transform[6 + axis] = right;
            }
        }
        true
    }

    pub fn push(&mut self, odf: &str, team: i32, position: [f32; 3], heading: [f32; 2]) -> Option<usize> {
        let [x, z] = heading;
        if !self.can_change_objects()
            || odf.is_empty()
            || odf.contains(&['/', '\\', '\0'][..])
            || !position.iter().all(|value| value.is_finite())
        {
            return None;
        }
        let id = self.next_entity_id?;
        self.next_entity_id = id.checked_add(1);
        let object = MapObject {
            odf: odf.to_owned(),
            entity_id: id,
            team,
            initial_position: Some(position),
            transform: [
                z, 0.0, -x,
                0.0, 1.0, 0.0,
                x, 0.0, z,
                position[0],
                position[1],
                position[2],
            ],
            ..Default::default()
        };
        self.objects.push(Some(object));
        Some(self.objects.len() - 1)
    }

    pub fn remove(&mut self, index: usize) -> bool {
        self.can_change_objects()
            && self
                .objects
                .get_mut(index)
                .is_some_and(|slot| slot.take().is_some())
    }

    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        let mut placements: Vec<MapObject> = self.objects.iter().flatten().cloned().collect();
        if self.path.to_ascii_lowercase().ends_with(".sol") {
            let original = sta_bzn::sol::parse(&self.source).map_err(|error| error.to_string())?;
            if original.to_map() != self.map {
                return Err("SOL map metadata cannot yet be saved".into());
            }
            return sta_bzn::sol::patch_placements(&self.source, &placements)
                .map_err(|error| error.to_string());
        }
        // A2 text maps have no native instance-name buffer. Its override is
        // kept in the STA64 editor data for this copy.
        if !self.map.binary && self.map.a1.is_none() {
            for object in &mut placements {
                object.instance_name.clear();
            }
        }
        let mut map = self.map.clone();
        map.objects = placements;
        let bytes = sta_bzn::write::write(&map).map_err(|error| error.to_string())?;
        let parsed = sta_bzn::parse(&bytes).map_err(|error| error.to_string())?;
        if parsed.objects != map.objects {
            return Err("saved map placements did not round trip".into());
        }
        Ok(bytes)
    }

    /// Write a new map next to the user's edits, never replacing an install
    /// file or an earlier saved copy.
    pub fn save_copy(&self, folder: &Path) -> Result<PathBuf, String> {
        let bytes = self.bytes()?;
        let properties: BTreeMap<_, _> = self.objects.iter().flatten().filter_map(|object| {
            self.properties.get(&object.entity_id).map(|value| (object.entity_id, value))
        }).collect();
        let metadata = if properties.is_empty() {
            None
        } else {
            let data = serde_json::json!({
                "version": 1,
                "map_checksum": checksum(&bytes),
                "objects": properties,
            });
            Some(serde_json::to_vec_pretty(&data).map_err(|error| error.to_string())?)
        };
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        let source = Path::new(self.path.strip_prefix("saved:").unwrap_or(&self.path));
        let stem = source
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or("map has no filename")?;
        let stem: String = stem
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        let extension = source
            .extension()
            .and_then(|name| name.to_str())
            .ok_or("map has no extension")?;
        if !matches!(extension.to_ascii_lowercase().as_str(), "bzn" | "sol") {
            return Err("unsupported map extension".into());
        }
        for number in 1..=10_000 {
            let suffix = if number == 1 {
                String::new()
            } else {
                format!("-{number}")
            };
            let path = folder.join(format!("{stem}-edit{suffix}.{extension}"));
            let companion = properties_file(&path);
            if companion.try_exists().map_err(|error| error.to_string())? {
                continue;
            }
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        return Err(error.to_string());
                    }
                    if let Some(bytes) = &metadata
                        && let Err(error) = write_properties(&companion, bytes)
                    {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(error);
                    }
                    return Ok(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("no free edited map filename".into())
    }
}

fn properties_file(map: &Path) -> PathBuf {
    let mut name = map.as_os_str().to_os_string();
    name.push(".sta64.json");
    PathBuf::from(name)
}

fn checksum(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

fn read_properties(map: &Path, bytes: &[u8]) -> Result<Option<BTreeMap<i32, ObjectProperties>>, String> {
    let companion = properties_file(map);
    match companion.symlink_metadata() {
        Ok(meta) if meta.file_type().is_file() => {}
        Ok(_) => return Err("The editor data for this map is not a regular file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    }
    const LIMIT: u64 = 8 * 1024 * 1024;
    let mut data = Vec::new();
    fs::File::open(companion).and_then(|file| file.take(LIMIT + 1).read_to_end(&mut data))
        .map_err(|error| error.to_string())?;
    if data.len() as u64 > LIMIT {
        return Err("The editor data for this map exceeds 8 MiB".into());
    }
    let mut parsed: serde_json::Value = serde_json::from_slice(&data).map_err(|error| error.to_string())?;
    let expected = checksum(bytes);
    if parsed["version"].as_u64() != Some(1) || parsed["map_checksum"].as_str() != Some(expected.as_str()) {
        return Err("The editor data does not match this map copy".into());
    }
    serde_json::from_value(parsed["objects"].take()).map(Some).map_err(|error| error.to_string())
}

fn write_properties(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path).map_err(|error| error.to_string())?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_properties_round_trip_with_their_map_copy() {
        let map = Map {
            version: 3013,
            size: [1_000.0, 100.0, 1_000.0],
            ..Default::default()
        };
        let source = sta_bzn::write::write(&map).unwrap();
        let mut doc = Document::new("bzn/test.bzn".into(), source, map).unwrap();
        let index = doc.push("fx_cruiser", 1, [12.0, 0.0, 35.0], [0.0, 1.0]).unwrap();
        let mut object = doc.object(index).unwrap().clone();
        object.instance_name = "Endeavour".into();
        let state = ObjectProperties {
            instance_name: Some("Endeavour".into()),
            captain: Some("Imani".into()),
            registry: Some(String::new()),
            hull: Some([Fixed::from_int(40), Fixed::from_int(80)]),
            ..Default::default()
        };
        doc.update_object(index, object, state.clone());
        let token = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("sta64-map-editor-{}-{token}", std::process::id()));
        let path = doc.save_copy(&folder).unwrap();
        let bytes = fs::read(&path).unwrap();
        let parsed = sta_bzn::parse(&bytes).unwrap();
        assert_eq!(parsed.objects[0].transform[9..12], [12.0, 0.0, 35.0]);
        assert_eq!(read_properties(&path, &bytes).unwrap().unwrap().get(&parsed.objects[0].entity_id), Some(&state));
        assert!(read_properties(&path, b"different BZN").is_err());
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn bzn_edits_keep_indices_and_round_trip_through_the_map_reader() {
        let map = Map {
            version: 3013,
            binary: false,
            description: "Editor fixture".into(),
            size: [1000.0, 100.0, 1000.0],
            objects: vec![MapObject {
                odf: "fyard".into(),
                entity_id: 1,
                team: 1,
                initial_position: Some([10.0, 0.0, 20.0]),
                transform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 0.0, 20.0],
                ..Default::default()
            }],
            ..Default::default()
        };
        let source = sta_bzn::write::write(&map).unwrap();
        let parsed = sta_bzn::parse(&source).unwrap();
        let mut document = Document::new("bzn/sample.bzn".into(), source, parsed).unwrap();
        assert!(document.move_to(0, [30.0, 0.0, 40.0]));
        assert!(document.rotate(0, 1));
        let added = document.push("fcruiser", 2, [50.0, 0.0, 60.0], [0.0, 1.0]).unwrap();
        assert!(document.remove(0));
        assert!(document.object(0).is_none());
        assert_eq!(document.object(added).unwrap().odf, "fcruiser");
        let reloaded = sta_bzn::parse(&document.bytes().unwrap()).unwrap();
        assert_eq!(reloaded.objects.len(), 1);
        assert_eq!(reloaded.objects[0].odf, "fcruiser");
        assert_eq!(&reloaded.objects[0].transform[9..12], &[50.0, 0.0, 60.0]);
    }
}
