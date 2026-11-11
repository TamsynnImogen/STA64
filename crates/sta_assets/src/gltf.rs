//! Static glTF 2.0 loader. All dependent bytes are resolved through the VFS.
use a2fmt::Matrix34;
use base64::Engine;
use sha2::{Digest, Sha256};
use sta_vfs::Vfs;

use crate::model::pbr::{
    PbrAlpha, PbrFilter, PbrImage, PbrImageFormat, PbrMaterial, PbrMinFilter, PbrModel, PbrSampler,
    PbrTexture, PbrWrap, VertexAttributes,
};
use crate::model::{Group, LightingMaterial, Maps, Mesh, Model, Node, NodeKind, SourceFormat};

fn resource_path(model: &str, uri: &str) -> Result<String, String> {
    if uri.is_empty()
        || uri.contains('\0')
        || uri.starts_with('/')
        || uri.starts_with('\\')
        || uri.contains('\\')
        || uri.contains('?')
        || uri.contains('#')
    {
        return Err(format!("unsafe resource URI {uri:?}"));
    }
    let mut decoded = Vec::new();
    let b = uri.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() {
                return Err(format!("bad percent escape in {uri:?}"));
            }
            let digit = |c: u8| (c as char).to_digit(16).map(|n| n as u8);
            let a = digit(b[i + 1]).ok_or_else(|| format!("bad percent escape in {uri:?}"))?;
            let z = digit(b[i + 2]).ok_or_else(|| format!("bad percent escape in {uri:?}"))?;
            decoded.push(a * 16 + z);
            i += 3;
        } else {
            decoded.push(b[i]);
            i += 1;
        }
    }
    let decoded =
        String::from_utf8(decoded).map_err(|_| format!("non-UTF8 resource URI {uri:?}"))?;
    if decoded.contains(['\0', '\\', '?', '#']) || decoded.starts_with('/') || decoded.contains(':')
    {
        return Err(format!("unsafe resource URI {uri:?}"));
    }
    let mut parts: Vec<&str> = model.split('/').filter(|x| !x.is_empty()).collect();
    parts.pop();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts
                    .pop()
                    .ok_or_else(|| format!("resource escapes VFS root: {uri:?}"))?;
            }
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(format!("empty resource path {uri:?}"));
    }
    Ok(parts.join("/"))
}

fn read_uri(vfs: &Vfs, path: &str, uri: &str) -> Result<(Vec<u8>, String), String> {
    if let Some(data) = uri.strip_prefix("data:") {
        let (header, payload) = data.split_once(',').ok_or("invalid data URI")?;
        if !header.ends_with(";base64") {
            return Err("only base64 data URIs are supported".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .map_err(|e| format!("data URI base64: {e}"))?;
        return Ok((bytes, format!("data URI ({header})")));
    }
    let resolved = resource_path(path, uri)?;
    let bytes = vfs
        .read(&resolved)
        .ok_or_else(|| format!("missing VFS resource {resolved}"))?
        .map_err(|e| format!("reading VFS resource {resolved}: {e}"))?;
    Ok((bytes, resolved))
}

fn preflight(doc: &gltf::Document, buffers: &[Vec<u8>]) -> Result<(), String> {
    for view in doc.views() {
        let end = view
            .offset()
            .checked_add(view.length())
            .ok_or("buffer view range overflow")?;
        if end > buffers[view.buffer().index()].len() {
            return Err(format!("buffer view {} exceeds buffer", view.index()));
        }
    }
    for accessor in doc.accessors() {
        // gltf::accessor::Iter subtracts one from count before the caller can
        // observe an empty iterator, so reject this malformed input first.
        if accessor.count() == 0 {
            return Err(format!("accessor {} has zero count", accessor.index()));
        }
        let element = accessor.size();
        if let Some(view) = accessor.view() {
            check_span(
                accessor.offset(),
                accessor.count(),
                view.stride().unwrap_or(element),
                element,
                view.length(),
                &format!("accessor {}", accessor.index()),
            )?;
        } else if accessor.sparse().is_none() {
            return Err(format!("accessor {} has no view", accessor.index()));
        }
        if let Some(sparse) = accessor.sparse() {
            if sparse.count() == 0 || sparse.count() > accessor.count() {
                return Err(format!(
                    "accessor {} invalid sparse count",
                    accessor.index()
                ));
            }
            let indices = sparse.indices();
            let index_view = indices.view();
            let width = indices.index_type().size();
            if indices.offset() % width != 0 || index_view.offset() % width != 0 {
                return Err(format!(
                    "accessor {} sparse index alignment",
                    accessor.index()
                ));
            }
            let stride = index_view.stride().unwrap_or(width);
            check_span(
                indices.offset(),
                sparse.count(),
                stride,
                width,
                index_view.length(),
                &format!("accessor {} sparse indices", accessor.index()),
            )?;
            let values = sparse.values();
            let value_view = values.view();
            let width_value = accessor.data_type().size();
            if values.offset() % width_value != 0 || value_view.offset() % width_value != 0 {
                return Err(format!(
                    "accessor {} sparse value alignment",
                    accessor.index()
                ));
            }
            check_span(
                values.offset(),
                sparse.count(),
                value_view.stride().unwrap_or(element),
                element,
                value_view.length(),
                &format!("accessor {} sparse values", accessor.index()),
            )?;
            let bytes = &buffers[index_view.buffer().index()];
            let mut previous = None;
            for i in 0..sparse.count() {
                let at = index_view.offset() + indices.offset() + i * stride;
                let index = match width {
                    1 => bytes[at] as u32,
                    2 => u16::from_le_bytes([bytes[at], bytes[at + 1]]) as u32,
                    4 => {
                        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
                    }
                    _ => return Err("unsupported sparse index width".into()),
                };
                if index as usize >= accessor.count() || previous.is_some_and(|p| index <= p) {
                    return Err(format!(
                        "accessor {} sparse indices out of range or unordered",
                        accessor.index()
                    ));
                }
                previous = Some(index);
            }
        }
    }
    Ok(())
}

fn check_span(
    offset: usize,
    count: usize,
    stride: usize,
    width: usize,
    view_len: usize,
    what: &str,
) -> Result<(), String> {
    if stride < width {
        return Err(format!("{what} stride smaller than element"));
    }
    let span = if count == 0 {
        0
    } else {
        (count - 1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(width))
            .ok_or_else(|| format!("{what} span overflow"))?
    };
    let end = offset
        .checked_add(span)
        .ok_or_else(|| format!("{what} range overflow"))?;
    if end > view_len {
        return Err(format!("{what} exceeds view"));
    }
    Ok(())
}

fn wrap(w: gltf::texture::WrappingMode) -> PbrWrap {
    use gltf::texture::WrappingMode::*;
    match w {
        ClampToEdge => PbrWrap::Clamp,
        MirroredRepeat => PbrWrap::Mirror,
        Repeat => PbrWrap::Repeat,
    }
}
fn texture(texture: gltf::Texture<'_>, tex_coord: u32) -> Result<PbrTexture, String> {
    if tex_coord > 1 {
        return Err(format!(
            "texture {} uses unsupported UV set {tex_coord}",
            texture.index()
        ));
    }
    let sampler = texture.sampler();
    use gltf::texture::{MagFilter, MinFilter};
    Ok(PbrTexture {
        image: texture.source().index(),
        tex_coord,
        sampler: PbrSampler {
            wrap_s: wrap(sampler.wrap_s()),
            wrap_t: wrap(sampler.wrap_t()),
            mag: sampler.mag_filter().map(|f| match f {
                MagFilter::Nearest => PbrFilter::Nearest,
                MagFilter::Linear => PbrFilter::Linear,
            }),
            min: sampler.min_filter().map(|f| match f {
                MinFilter::Nearest => PbrMinFilter::Nearest,
                MinFilter::Linear => PbrMinFilter::Linear,
                MinFilter::NearestMipmapNearest => PbrMinFilter::NearestMipmapNearest,
                MinFilter::LinearMipmapNearest => PbrMinFilter::LinearMipmapNearest,
                MinFilter::NearestMipmapLinear => PbrMinFilter::NearestMipmapLinear,
                MinFilter::LinearMipmapLinear => PbrMinFilter::LinearMipmapLinear,
            }),
        },
    })
}
fn material(src: gltf::Material<'_>) -> Result<PbrMaterial, String> {
    let p = src.pbr_metallic_roughness();
    let alpha = match src.alpha_mode() {
        gltf::material::AlphaMode::Opaque => PbrAlpha::Opaque,
        gltf::material::AlphaMode::Mask => PbrAlpha::Mask(src.alpha_cutoff().unwrap_or(0.5)),
        gltf::material::AlphaMode::Blend => PbrAlpha::Blend,
    };
    let mut m = PbrMaterial {
        name: src.name().map(str::to_owned),
        base_color: p.base_color_factor(),
        metallic: p.metallic_factor(),
        roughness: p.roughness_factor(),
        emissive: src.emissive_factor(),
        alpha,
        double_sided: src.double_sided(),
        unlit: src.unlit(),
        ..Default::default()
    };
    m.base_color_texture = p
        .base_color_texture()
        .map(|t| texture(t.texture(), t.tex_coord()))
        .transpose()?;
    m.metallic_roughness_texture = p
        .metallic_roughness_texture()
        .map(|t| texture(t.texture(), t.tex_coord()))
        .transpose()?;
    m.emissive_texture = src
        .emissive_texture()
        .map(|t| texture(t.texture(), t.tex_coord()))
        .transpose()?;
    if let Some(t) = src.normal_texture() {
        m.normal_scale = t.scale();
        m.normal_texture = Some(texture(t.texture(), t.tex_coord())?);
    }
    if let Some(t) = src.occlusion_texture() {
        m.occlusion_strength = t.strength();
        m.occlusion_texture = Some(texture(t.texture(), t.tex_coord())?);
    }
    if !m
        .base_color
        .iter()
        .chain(m.emissive.iter())
        .chain(
            [
                m.metallic,
                m.roughness,
                m.normal_scale,
                m.occlusion_strength,
            ]
            .iter(),
        )
        .all(|x| x.is_finite())
        || matches!(m.alpha, PbrAlpha::Mask(x) if !x.is_finite())
    {
        return Err("nonfinite material factor".into());
    }
    Ok(m)
}

fn transform(src: gltf::scene::Transform) -> Result<Matrix34, String> {
    let m = src.matrix();
    let out = Matrix34 {
        m: [
            m[0][0], m[0][1], -m[0][2], m[1][0], m[1][1], -m[1][2], -m[2][0], -m[2][1], m[2][2],
            m[3][0], m[3][1], -m[3][2],
        ],
    };
    if !out.is_finite() {
        return Err("nonfinite node transform".into());
    }
    Ok(out)
}

fn primitive(
    src: gltf::Primitive<'_>,
    buffers: &[Vec<u8>],
) -> Result<(Mesh, VertexAttributes), String> {
    use gltf::mesh::Mode;
    if src.morph_targets().next().is_some() {
        return Err("morph targets unsupported".into());
    }
    if !matches!(
        src.mode(),
        Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan
    ) {
        return Err(format!("primitive mode {:?} unsupported", src.mode()));
    }
    use gltf::accessor::{DataType as T, Dimensions as D};
    for (semantic, accessor) in src.attributes() {
        let shape = (
            accessor.data_type(),
            accessor.dimensions(),
            accessor.normalized(),
        );
        let valid = match semantic {
            gltf::Semantic::Positions | gltf::Semantic::Normals => {
                shape == (T::F32, D::Vec3, false)
            }
            gltf::Semantic::Tangents => shape == (T::F32, D::Vec4, false),
            gltf::Semantic::TexCoords(0 | 1) => matches!(
                shape,
                (T::F32, D::Vec2, false) | (T::U8 | T::U16, D::Vec2, true)
            ),
            gltf::Semantic::Colors(0) => matches!(
                shape,
                (T::F32, D::Vec3 | D::Vec4, false) | (T::U8 | T::U16, D::Vec3 | D::Vec4, true)
            ),
            _ => return Err(format!("primitive attribute {semantic:?} unsupported")),
        };
        if !valid {
            return Err(format!(
                "primitive attribute {semantic:?} accessor {} has invalid component type, shape or normalization",
                accessor.index()
            ));
        }
    }
    if let Some(indices) = src.indices()
        && (indices.dimensions() != D::Scalar
            || !matches!(indices.data_type(), T::U8 | T::U16 | T::U32)
            || indices.normalized())
    {
        return Err(format!(
            "primitive indices accessor {} has invalid component type, shape or normalization",
            indices.index()
        ));
    }
    let reader = src.reader(|b| Some(buffers[b.index()].as_slice()));
    let positions: Vec<_> = reader
        .read_positions()
        .ok_or("primitive missing POSITION")?
        .map(|[x, y, z]| [x, y, -z])
        .collect();
    if positions.is_empty() || !positions.iter().flatten().all(|x| x.is_finite()) {
        return Err("empty/nonfinite positions".into());
    }
    let count = positions.len();
    let read_uv = |set| -> Result<Option<Vec<[f32; 2]>>, String> {
        reader
            .read_tex_coords(set)
            .map(|v| {
                let a: Vec<_> = v.into_f32().collect();
                if a.len() != count || !a.iter().flatten().all(|x| x.is_finite()) {
                    Err(format!("TEXCOORD_{set} count/nonfinite mismatch"))
                } else {
                    Ok(a)
                }
            })
            .transpose()
    };
    let uvs = read_uv(0)?.unwrap_or_else(|| vec![[0.0, 0.0]; count]);
    let uv1 = read_uv(1)?;
    let normals = reader
        .read_normals()
        .map(|v| v.map(|[x, y, z]| [x, y, -z]).collect::<Vec<_>>());
    let tangents = reader
        .read_tangents()
        .map(|v| v.map(|[x, y, z, w]| [x, y, -z, -w]).collect::<Vec<_>>());
    let colors = reader
        .read_colors(0)
        .map(|v| v.into_rgba_f32().collect::<Vec<_>>());
    if normals
        .as_ref()
        .is_some_and(|v| v.len() != count || !v.iter().flatten().all(|x| x.is_finite()))
        || tangents
            .as_ref()
            .is_some_and(|v| v.len() != count || !v.iter().flatten().all(|x| x.is_finite()))
        || colors
            .as_ref()
            .is_some_and(|v| v.len() != count || !v.iter().flatten().all(|x| x.is_finite()))
    {
        return Err("vertex channel count/nonfinite mismatch".into());
    }
    if count > u32::MAX as usize {
        return Err("vertex count exceeds u32 index space".into());
    }
    let indices: Vec<u32> = reader
        .read_indices()
        .map(|v| v.into_u32().collect())
        .unwrap_or_else(|| (0..count as u32).collect());
    if indices.iter().any(|&i| i as usize >= count) {
        return Err("index exceeds vertex count".into());
    }
    let mut faces = Vec::new();
    match src.mode() {
        Mode::Triangles => {
            if !indices.len().is_multiple_of(3) {
                return Err("triangle index count not multiple of 3".into());
            }
            for c in indices.chunks_exact(3) {
                faces.push([c[0], c[1], c[2]].map(|i| (i, i)));
            }
        }
        Mode::TriangleStrip => {
            for i in 2..indices.len() {
                let a = if i % 2 == 0 {
                    [indices[i - 2], indices[i - 1], indices[i]]
                } else {
                    [indices[i - 1], indices[i - 2], indices[i]]
                };
                if a[0] != a[1] && a[1] != a[2] && a[0] != a[2] {
                    faces.push(a.map(|j| (j, j)));
                }
            }
        }
        Mode::TriangleFan => {
            for i in 2..indices.len() {
                let a = [indices[0], indices[i - 1], indices[i]];
                if a[0] != a[1] && a[1] != a[2] && a[0] != a[2] {
                    faces.push(a.map(|j| (j, j)));
                }
            }
        }
        _ => unreachable!(),
    }
    let mat = src.material();
    let p = mat.pbr_metallic_roughness();
    let texture_sets = [
        p.base_color_texture().map(|t| t.tex_coord()),
        p.metallic_roughness_texture().map(|t| t.tex_coord()),
        mat.normal_texture().map(|t| t.tex_coord()),
        mat.emissive_texture().map(|t| t.tex_coord()),
        mat.occlusion_texture().map(|t| t.tex_coord()),
    ];
    if texture_sets.contains(&Some(1)) && uv1.is_none() {
        return Err("material needs missing TEXCOORD_1".into());
    }
    if texture_sets.contains(&Some(0)) && src.get(&gltf::Semantic::TexCoords(0)).is_none() {
        return Err("material needs missing TEXCOORD_0".into());
    }
    let key = mat
        .index()
        .map_or("gltf_default".to_owned(), |i| format!("gltf_{i}"));
    let group = Group {
        material: key,
        blend: "default".into(),
        maps: Maps::default(),
        faces,
    };
    Ok((
        Mesh {
            positions,
            uvs,
            groups: vec![group],
            two_sided: mat.double_sided(),
        },
        VertexAttributes {
            normals,
            tangents,
            colors,
            uv1,
        },
    ))
}

fn visit_scene(
    scene: gltf::Scene<'_>,
    state: &mut [u8],
    model: &mut Model,
    buffers: &[Vec<u8>],
) -> Result<(), String> {
    // Iterative DFS avoids stack exhaustion on a hostile deep hierarchy.
    let mut stack: Vec<_> = scene.nodes().map(|n| (n, None, false)).collect();
    stack.reverse();
    while let Some((node, parent, leaving)) = stack.pop() {
        let id = node.index();
        if leaving {
            state[id] = 2;
            continue;
        }
        if state[id] != 0 {
            return Err(format!("node {id} cycle or multiple parents"));
        }
        state[id] = 1;
        if node.skin().is_some() || node.weights().is_some() {
            return Err(format!("node {id} skin/morph weights unsupported"));
        }
        let index = model.nodes.len();
        model.nodes.push(Node {
            name: node
                .name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("node_{id}")),
            parent,
            children: vec![],
            transform: transform(node.transform())?,
            kind: NodeKind::Null,
        });
        if let Some(p) = parent {
            model.nodes[p].children.push(index);
        }
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                let primitive_index = prim.index();
                let (geometry, attributes) = primitive(prim, buffers).map_err(|e| {
                    format!(
                        "node {id} mesh {} primitive {primitive_index}: {e}",
                        mesh.index()
                    )
                })?;
                let child = model.nodes.len();
                model.nodes.push(Node {
                    name: format!("{}_primitive_{primitive_index}", model.nodes[index].name),
                    parent: Some(index),
                    children: vec![],
                    transform: Matrix34::IDENTITY,
                    kind: NodeKind::Mesh(geometry),
                });
                model.nodes[index].children.push(child);
                model
                    .pbr
                    .as_mut()
                    .unwrap()
                    .attributes
                    .insert(child, attributes);
            }
        }
        stack.push((node.clone(), parent, true));
        let mut children: Vec<_> = node
            .children()
            .map(|child| (child, Some(index), false))
            .collect();
        children.reverse();
        stack.extend(children);
    }
    Ok(())
}

/// Load a static glTF 2.0 or GLB scene from the layered virtual filesystem.
pub fn load(vfs: &Vfs, path: &str) -> Result<Model, String> {
    let bytes = vfs
        .read(path)
        .ok_or_else(|| format!("missing VFS model {path}"))?
        .map_err(|e| format!("reading {path}: {e}"))?;
    let gltf = gltf::Gltf::from_slice(&bytes).map_err(|e| format!("parsing {path}: {e}"))?;
    for ext in gltf.extensions_required() {
        if ext != "KHR_materials_unlit" {
            return Err(format!("required extension {ext} unsupported"));
        }
    }
    let mut warnings = vec![];
    for ext in gltf.extensions_used() {
        if ext != "KHR_materials_unlit" {
            warnings.push(format!("optional extension {ext} uses core fallback"));
        }
    }
    if gltf.animations().next().is_some() || gltf.skins().next().is_some() {
        return Err("animations/skins unsupported".into());
    }
    let mut buffers = Vec::new();
    for buffer in gltf.buffers() {
        let mut data = match buffer.source() {
            gltf::buffer::Source::Bin => gltf.blob.as_ref().ok_or("GLB missing BIN chunk")?.clone(),
            gltf::buffer::Source::Uri(uri) => read_uri(vfs, path, uri)?.0,
        };
        if data.len() < buffer.length() {
            return Err(format!(
                "buffer {} shorter than declared length",
                buffer.index()
            ));
        }
        data.truncate(buffer.length());
        buffers.push(data);
    }
    preflight(&gltf, &buffers)?;
    let mut pbr = PbrModel::default();
    for image in gltf.images() {
        let (data, origin, mime) = match image.source() {
            gltf::image::Source::Uri { uri, mime_type } => {
                let (data, origin) = read_uri(vfs, path, uri)?;
                let data_mime = uri
                    .strip_prefix("data:")
                    .and_then(|s| s.split_once(','))
                    .map(|(header, _)| header.strip_suffix(";base64").unwrap_or(header));
                if let (Some(declared), Some(inline)) = (mime_type, data_mime)
                    && declared != inline
                {
                    return Err(format!(
                        "image {} data URI MIME conflicts with declared MIME",
                        image.index()
                    ));
                }
                (data, origin, mime_type.or(data_mime).map(str::to_owned))
            }
            gltf::image::Source::View { view, mime_type } => {
                let b = &buffers[view.buffer().index()];
                (
                    b[view.offset()..view.offset() + view.length()].to_vec(),
                    format!("buffer view {}", view.index()),
                    Some(mime_type.to_owned()),
                )
            }
        };
        let format = match mime.as_deref() {
            Some("image/png") => PbrImageFormat::Png,
            Some("image/jpeg") => PbrImageFormat::Jpeg,
            None if data.starts_with(b"\x89PNG\r\n\x1a\n") => PbrImageFormat::Png,
            None if data.starts_with(b"\xff\xd8\xff") => PbrImageFormat::Jpeg,
            _ => {
                return Err(format!(
                    "image {} unsupported MIME or encoding",
                    image.index()
                ));
            }
        };
        let signature_ok = match format {
            PbrImageFormat::Png => data.starts_with(b"\x89PNG\r\n\x1a\n"),
            PbrImageFormat::Jpeg => data.starts_with(b"\xff\xd8\xff"),
        };
        if !signature_ok {
            return Err(format!(
                "image {} encoding does not match MIME",
                image.index()
            ));
        }
        pbr.images.push(PbrImage {
            name: image.name().map(str::to_owned),
            origin,
            format,
            content_hash: Sha256::digest(&data).into(),
            data,
        });
    }
    for src in gltf.materials() {
        pbr.materials
            .insert(format!("gltf_{}", src.index().unwrap()), material(src)?);
    }
    pbr.materials
        .insert("gltf_default".into(), PbrMaterial::default());
    let scene = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .ok_or("glTF has no scene")?;
    let mut model = Model {
        format: SourceFormat::Gltf,
        version: 2.0,
        materials: Vec::<LightingMaterial>::new(),
        nodes: vec![],
        channels: vec![],
        flipbooks: vec![],
        warnings,
        pbr: Some(pbr),
    };
    let mut state = vec![0; gltf.nodes().count()];
    visit_scene(scene, &mut state, &mut model, &buffers)?;
    Ok(model)
}

#[cfg(test)]
#[path = "gltf/tests.rs"]
mod tests;
