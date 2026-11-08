//! CPU DDS decoding to RGBA8: DXT1/2/3/4/5 and uncompressed bitmask formats.
//!
//! Fleet Operations ships DXT1, DXT3, DXT5 and uncompressed (24/32-bit) DDS.
//! The GPU takes block-compressed files directly where it can. This decoder
//! covers uncompressed files, height maps (read on the CPU to build normal
//! maps), and GPUs without BC support.

use crate::Rgba;
use ddsfile::{Dds, FourCC};

#[derive(Debug, PartialEq, Eq)]
pub enum DdsError {
    Parse(String),
    Unsupported(String),
    Truncated,
}

impl std::fmt::Display for DdsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(s) => write!(f, "parse error: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported format: {s}"),
            Self::Truncated => write!(f, "truncated"),
        }
    }
}

/// Kind of pixel data in a DDS file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsKind {
    Bc1,
    Bc2,
    Bc3,
    Uncompressed,
}

pub fn kind(data: &[u8]) -> Result<DdsKind, DdsError> {
    let dds = Dds::read(data).map_err(|e| DdsError::Parse(e.to_string()))?;
    kind_of(&dds)
}

fn kind_of(dds: &Dds) -> Result<DdsKind, DdsError> {
    match dds.header.spf.fourcc.as_ref().map(|f| f.0) {
        None | Some(FourCC::NONE) => Ok(DdsKind::Uncompressed),
        Some(FourCC::DXT1) => Ok(DdsKind::Bc1),
        Some(FourCC::DXT2 | FourCC::DXT3) => Ok(DdsKind::Bc2),
        Some(FourCC::DXT4 | FourCC::DXT5) => Ok(DdsKind::Bc3),
        Some(other) => Err(DdsError::Unsupported(format!("fourcc {:?}", other.to_le_bytes().map(char::from)))),
    }
}

/// Decode the top mip level.
pub fn decode(data: &[u8]) -> Result<Rgba, DdsError> {
    let dds = Dds::read(data).map_err(|e| DdsError::Parse(e.to_string()))?;
    let (w, h) = (dds.get_width(), dds.get_height());
    if w == 0 || h == 0 {
        return Err(DdsError::Truncated);
    }
    // The top level starts right after the header(s); later levels follow and
    // are ignored here. (`get_data` refuses some uncompressed layouts.)
    let src = dds.data.as_slice();
    let kind = kind_of(&dds)?;
    let pixels = match kind {
        DdsKind::Bc1 | DdsKind::Bc2 | DdsKind::Bc3 => decode_bc(kind, src, w, h)?,
        DdsKind::Uncompressed => decode_masked(&dds, src, w, h)?,
    };
    let has_alpha = kind != DdsKind::Bc1 || pixels.chunks_exact(4).any(|p| p[3] != 255);
    Ok(Rgba {
        width: w,
        height: h,
        has_alpha,
        pixels,
    })
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [((r * 255 + 15) / 31) as u8, ((g * 255 + 31) / 63) as u8, ((b * 255 + 15) / 31) as u8]
}

/// Colour endpoints and the 2-bit selectors of one BC1-style colour block.
fn colour_block(b: &[u8], force_four: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mix = |a: u8, b: u8, wa: u32, wb: u32| ((a as u32 * wa + b as u32 * wb) / (wa + wb)) as u8;
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [p0[0], p0[1], p0[2], 255];
    pal[1] = [p1[0], p1[1], p1[2], 255];
    if c0 > c1 || force_four {
        pal[2] = [mix(p0[0], p1[0], 2, 1), mix(p0[1], p1[1], 2, 1), mix(p0[2], p1[2], 2, 1), 255];
        pal[3] = [mix(p0[0], p1[0], 1, 2), mix(p0[1], p1[1], 1, 2), mix(p0[2], p1[2], 1, 2), 255];
    } else {
        pal[2] = [mix(p0[0], p1[0], 1, 1), mix(p0[1], p1[1], 1, 1), mix(p0[2], p1[2], 1, 1), 255];
        pal[3] = [0, 0, 0, 0];
    }
    let sel = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    std::array::from_fn(|i| pal[((sel >> (2 * i)) & 3) as usize])
}

fn decode_bc(kind: DdsKind, src: &[u8], w: u32, h: u32) -> Result<Vec<u8>, DdsError> {
    let (bw, bh) = (w.div_ceil(4) as usize, h.div_ceil(4) as usize);
    let block = if kind == DdsKind::Bc1 { 8 } else { 16 };
    if src.len() < bw * bh * block {
        return Err(DdsError::Truncated);
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    for by in 0..bh {
        for bx in 0..bw {
            let b = &src[(by * bw + bx) * block..][..block];
            let mut texels = match kind {
                DdsKind::Bc1 => colour_block(b, false),
                _ => colour_block(&b[8..], true),
            };
            match kind {
                DdsKind::Bc2 => {
                    for (i, t) in texels.iter_mut().enumerate() {
                        let nib = (b[i / 2] >> ((i % 2) * 4)) & 0xF;
                        t[3] = nib * 17;
                    }
                }
                DdsKind::Bc3 => {
                    let (a0, a1) = (b[0] as u32, b[1] as u32);
                    let mut pal = [0u8; 8];
                    pal[0] = a0 as u8;
                    pal[1] = a1 as u8;
                    if a0 > a1 {
                        for i in 1..7 {
                            pal[i + 1] = (((7 - i as u32) * a0 + i as u32 * a1) / 7) as u8;
                        }
                    } else {
                        for i in 1..5 {
                            pal[i + 1] = (((5 - i as u32) * a0 + i as u32 * a1) / 5) as u8;
                        }
                        pal[6] = 0;
                        pal[7] = 255;
                    }
                    let bits = u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]);
                    for (i, t) in texels.iter_mut().enumerate() {
                        t[3] = pal[((bits >> (3 * i)) & 7) as usize];
                    }
                }
                _ => {}
            }
            for (i, t) in texels.iter().enumerate() {
                let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                if x < w as usize && y < h as usize {
                    out[(y * w as usize + x) * 4..][..4].copy_from_slice(t);
                }
            }
        }
    }
    Ok(out)
}

fn decode_masked(dds: &Dds, src: &[u8], w: u32, h: u32) -> Result<Vec<u8>, DdsError> {
    let pf = &dds.header.spf;
    let bits = pf.rgb_bit_count.ok_or_else(|| DdsError::Unsupported("no bit count".into()))?;
    if !matches!(bits, 8 | 16 | 24 | 32) {
        return Err(DdsError::Unsupported(format!("{bits} bits per pixel")));
    }
    let bpp = (bits / 8) as usize;
    let luminance = pf.flags.contains(ddsfile::PixelFormatFlags::LUMINANCE);
    let r = pf.r_bit_mask.unwrap_or(0);
    let g = pf.g_bit_mask.unwrap_or(0);
    let b = pf.b_bit_mask.unwrap_or(0);
    let a = if pf.flags.intersects(ddsfile::PixelFormatFlags::ALPHA_PIXELS | ddsfile::PixelFormatFlags::ALPHA) {
        pf.a_bit_mask.unwrap_or(0)
    } else {
        0
    };
    let channel = |v: u32, mask: u32| -> Option<u8> {
        if mask == 0 {
            return None;
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        Some((((v & mask) >> shift) * 255 / max) as u8)
    };
    // Rows may be padded to a pitch; trust the header pitch when it is sane.
    let row = (w as usize) * bpp;
    let pitch = dds.get_pitch().map(|p| p as usize).filter(|&p| p >= row).unwrap_or(row);
    if src.len() < pitch * (h as usize - 1) + row {
        return Err(DdsError::Truncated);
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let s = &src[y * pitch + x * bpp..][..bpp];
            let mut v = 0u32;
            for (i, byte) in s.iter().enumerate() {
                v |= (*byte as u32) << (8 * i);
            }
            let px = if luminance {
                let l = channel(v, r).unwrap_or(0);
                [l, l, l, channel(v, a).unwrap_or(255)]
            } else {
                [
                    channel(v, r).unwrap_or(0),
                    channel(v, g).unwrap_or(0),
                    channel(v, b).unwrap_or(0),
                    channel(v, a).unwrap_or(255),
                ]
            };
            out[(y * w as usize + x) * 4..][..4].copy_from_slice(&px);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc1_block_endpoints_and_transparency() {
        // c0 = pure red (0xF800) > c1 = pure blue (0x001F): four-colour mode.
        let b = [0x00, 0xF8, 0x1F, 0x00, 0b10_11_01_00, 0, 0, 0];
        let t = colour_block(&b, false);
        assert_eq!(t[0], [255, 0, 0, 255]);
        assert_eq!(t[1], [0, 0, 255, 255]);
        // c0 <= c1: three colours + transparent black at index 3.
        let b = [0x1F, 0x00, 0x00, 0xF8, 0b11, 0, 0, 0];
        assert_eq!(colour_block(&b, false)[0], [0, 0, 0, 0]);
    }
}
