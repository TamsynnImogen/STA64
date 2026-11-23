//! An editable map document. Indices stay stable while objects are removed so
//! selection and undo records can refer to the same placement.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use bevy::prelude::Resource;
use sta_bzn::{Map, MapObject};

#[derive(Resource, Clone)]
pub struct Document {
    pub path: String,
    pub map: Map,
    pub objects: Vec<Option<MapObject>>,
    source: Vec<u8>,
    next_entity_id: Option<i32>,
}

impl Document {
    pub fn new(path: String, bytes: Vec<u8>, map: Map) -> Result<Self, String> {
        if path.to_ascii_lowercase().ends_with(".sol") {
            let original = sta_bzn::sol::parse(&bytes).map_err(|error| error.to_string())?;
            if original.to_map() != map {
                return Err("SOL map does not match its source bytes".into());
            }
        }
        let objects = map.objects.iter().cloned().map(Some).collect();
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
            source: bytes,
            next_entity_id,
        })
    }

    pub fn object(&self, index: usize) -> Option<&MapObject> {
        self.objects.get(index)?.as_ref()
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
        let placements: Vec<MapObject> = self.objects.iter().flatten().cloned().collect();
        if self.path.to_ascii_lowercase().ends_with(".sol") {
            let original = sta_bzn::sol::parse(&self.source).map_err(|error| error.to_string())?;
            if original.to_map() != self.map {
                return Err("SOL map metadata cannot yet be saved".into());
            }
            return sta_bzn::sol::patch_placements(&self.source, &placements)
                .map_err(|error| error.to_string());
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
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        return Err(error.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

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
