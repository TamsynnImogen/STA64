//! Explicit Legacy chunk contexts, separate from complete scene models.
//!
//! Material libraries and geometry variants share the scene magic/version but
//! have different bodies. Callers select the role; these decoders never guess,
//! rewrite assets, load textures, or emulate native destruction/variant choice.
use super::{Reader, text};

/// Raw texture binding. Preserve spelling and flag bits for the owning system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureBinding {
    pub reference: Vec<u8>,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkMaterial {
    pub lighting_material: Vec<u8>,
    /// Diffuse, bump, specular, glow and detail, in native stream order.
    pub textures: [TextureBinding; 5],
    pub shader: Vec<u8>,
    pub extra_shader: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterialLibrary {
    pub version: f32,
    pub materials: Vec<ChunkMaterial>,
    /// Number of numbered geometry variants, not a complete-model node count.
    pub variant_count: u32,
}

/// Native left-handed corner binding and winding, before scene conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Corner {
    pub position: u32,
    pub normal: u32,
    pub uv: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkGroup {
    /// Negative values select the native default; nonnegative values index the
    /// separate material library. No library is silently synthesized here.
    pub material_index: i32,
    pub faces: Vec<[Corner; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkMesh {
    pub special_flag: u32,
    pub flags: u32,
    pub trailer: u16,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub groups: Vec<ChunkGroup>,
    pub cull: i8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeometryVariant {
    pub version: f32,
    /// Two native bound-vector/scalar records, preserved without interpreting
    /// their values as scene transforms or inventing placement defaults.
    pub bounds: [f32; 8],
    pub chunks: Vec<ChunkMesh>,
}

fn header(reader: &mut Reader<'_>) -> Option<f32> {
    reader.strict_numbers = true;
    let magic = reader.raw(10, "chunk header")?;
    if &magic[..8] != b"Mad3D_SW" && magic != b"Storm3D_SW" && magic != b"StarTrekDB" {
        return reader.fail("Not a native Legacy chunk stream.");
    }
    let version = reader.f32("chunk version")?;
    if ![2.04_f32, 2.05, 2.06].contains(&version) {
        return reader.fail(&format!(
            "Unsupported Legacy chunk version {version}; expected 2.04, 2.05 or 2.06."
        ));
    }
    Some(version)
}

fn finish(reader: &mut Reader<'_>) -> Option<()> {
    if reader.remaining() != 0 {
        return reader.fail(&format!("Unrecognized chunk trailing data at byte {}.", reader.at));
    }
    Some(())
}

/// Decode a material library using its explicit native context.
pub fn load_materials(input: &[u8]) -> Result<MaterialLibrary, String> {
    let mut reader = Reader::new(input);
    let result = (|| {
        let version = header(&mut reader)?;
        let count = reader.u32("chunk material count")? as usize;
        // Empty names still need five length/flag pairs, the shader strings,
        // and the final variant word. Check before allocating a forged table.
        let minimum_record = if version >= 2.05 { 36 } else { 34 };
        if count > reader.remaining().saturating_sub(4) / minimum_record {
            return reader.fail("Invalid chunk material count.");
        }
        let mut materials = Vec::with_capacity(count);
        for _ in 0..count {
            let group = reader.material_record(version)?;
            let references = [group.diffuse, group.bump, group.specular, group.glow, group.detail];
            let mut references = references.into_iter().zip(group.texture_flags);
            materials.push(ChunkMaterial {
                lighting_material: group.material,
                textures: std::array::from_fn(|_| {
                    let (reference, flags) = references.next().expect("five native texture slots");
                    TextureBinding { reference, flags }
                }),
                shader: group.shader,
                extra_shader: group.extra_shader,
            });
        }
        let variant_count = reader.u32("geometry variant count")?;
        finish(&mut reader)?;
        Some(MaterialLibrary {
            version,
            materials,
            variant_count,
        })
    })();
    result.ok_or_else(|| format!("chunk material library: {}", text(&reader.error)))
}

/// Decode geometry while retaining external material indices and native flags.
pub fn load_geometry(input: &[u8]) -> Result<GeometryVariant, String> {
    let mut reader = Reader::new(input);
    let result = (|| {
        let version = header(&mut reader)?;
        let mut bounds = [0.0; 8];
        for value in &mut bounds {
            *value = reader.f32("chunk bounds")?;
        }
        let count = reader.u32("chunk mesh count")? as usize;
        if count > reader.remaining() / 19 {
            return reader.fail("Invalid chunk mesh count.");
        }
        // Shipped exports contain NaN/Infinity attributes. This is a raw asset
        // decoder, not a GPU adapter: retain their bits and expose warnings.
        // Bounds above remain finite; structural/index checks below still apply.
        reader.strict_numbers = false;
        reader.preserve_numbers = true;
        let mut chunks = Vec::with_capacity(count);
        for _ in 0..count {
            let special_flag = reader.u32("chunk special flag")?;
            let mesh = reader.mesh_with_materials(version, false)?;
            let mut groups = Vec::with_capacity(mesh.groups.len());
            for group in mesh.groups {
                let material_index = reader.u32("chunk material index")? as i32;
                let mut faces = Vec::with_capacity(group.faces.len());
                for face in group.faces {
                    if face.positions.iter().any(|&i| i as usize >= mesh.vertices.len())
                        || face.uvs.iter().any(|&i| i as usize >= mesh.uvs.len())
                        || (mesh.flags & 0x10 != 0 && face.normals.iter().any(|&i| i as usize >= mesh.normals.len()))
                    {
                        return reader.fail("Chunk face has an out-of-range attribute index.");
                    }
                    faces.push(std::array::from_fn(|i| Corner {
                        position: face.positions[i],
                        normal: face.normals[i],
                        uv: face.uvs[i],
                    }));
                }
                groups.push(ChunkGroup { material_index, faces });
            }
            chunks.push(ChunkMesh {
                special_flag,
                flags: mesh.flags,
                trailer: mesh.trailer,
                positions: mesh.vertices,
                normals: mesh.normals,
                uvs: mesh.uvs,
                groups,
                cull: mesh.cull,
            });
        }
        finish(&mut reader)?;
        Some(GeometryVariant {
            version,
            bounds,
            chunks,
        })
    })();
    result.ok_or_else(|| format!("chunk geometry: {}", text(&reader.error)))
}

impl GeometryVariant {
    /// Raw attributes are not automatically safe to upload. Report malformed
    /// stored values without changing the author's bytes or inventing UVs.
    pub fn attribute_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        for (index, chunk) in self.chunks.iter().enumerate() {
            let bad_positions = chunk.positions.iter().flatten().filter(|v| !v.is_finite()).count();
            let bad_normals = chunk.normals.iter().flatten().filter(|v| !v.is_finite()).count();
            let bad_uvs = chunk.uvs.iter().flatten().filter(|v| !v.is_finite()).count();
            for (role, count) in [("position", bad_positions), ("normal", bad_normals), ("UV", bad_uvs)] {
                if count != 0 {
                    warnings.push(format!(
                        "chunk {index}: {count} non-finite {role} components retained; not render-safe"
                    ));
                }
            }
        }
        warnings
    }

    /// Validate bindings when the caller has supplied the actual library.
    pub fn validate_materials(&self, library: &MaterialLibrary) -> Result<(), String> {
        for (chunk, mesh) in self.chunks.iter().enumerate() {
            for (group, data) in mesh.groups.iter().enumerate() {
                if data.material_index >= 0 && data.material_index as usize >= library.materials.len() {
                    return Err(format!(
                        "chunk {chunk} group {group} material {} is outside library of {} records",
                        data.material_index,
                        library.materials.len()
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
