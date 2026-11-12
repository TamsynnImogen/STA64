//! Windows/OS2 BMP decoding to RGBA8 (CON-21): menu and shell images of all
//! three games (splash screens, ScrnThms, bitmaps/).
//!
//! Supported: 1/4/8-bit palette images (plain and RLE4/RLE8), 16-bit (X1R5G5B5
//! or bit fields), 24-bit and 32-bit (plain or bit fields); OS/2 core headers
//! (12 bytes) and the 40/52/56/64/108/124-byte info headers; bottom-up and
//! top-down rows. Output rows are top-down, like every other decoder here.

use crate::{AssetError, Rgba, read};
use sta_vfs::Vfs;

/// Why a BMP could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BmpError {
    /// Not a `BM` file, or shorter than its headers.
    Header(&'static str),
    /// A bit depth / compression pair this decoder does not read (JPEG/PNG BMPs).
    Unsupported(String),
    /// The pixel data runs past the end of the file or the RLE stream is malformed.
    Truncated,
    /// Width, height or the resulting buffer is zero or absurdly large.
    Dimensions(i64, i64),
}

impl std::fmt::Display for BmpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Header(m) => write!(f, "bad header: {m}"),
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
            Self::Truncated => write!(f, "truncated or malformed pixel data"),
            Self::Dimensions(w, h) => write!(f, "bad dimensions {w}x{h}"),
        }
    }
}

impl std::error::Error for BmpError {}

/// Largest edge accepted; the biggest shipped BMPs are a few thousand pixels.
const MAX_EDGE: i64 = 16384;

const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;

fn u16le(d: &[u8], at: usize) -> Result<u16, BmpError> {
    d.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or(BmpError::Header("short file"))
}

fn u32le(d: &[u8], at: usize) -> Result<u32, BmpError> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(BmpError::Header("short file"))
}

/// One colour channel described by a bit mask.
#[derive(Clone, Copy)]
struct Channel {
    shift: u32,
    bits: u32,
}

impl Channel {
    fn from_mask(mask: u32) -> Option<Self> {
        if mask == 0 {
            return None;
        }
        let shift = mask.trailing_zeros();
        let bits = (mask >> shift).trailing_ones();
        // Masks wider than 8 bits keep their top 8 bits.
        Some(if bits > 8 { Self { shift: shift + bits - 8, bits: 8 } } else { Self { shift, bits } })
    }

    fn get(self, v: u32) -> u8 {
        let x = (v >> self.shift) & ((1u32 << self.bits) - 1);
        // Replicate the high bits so full scale maps to 255.
        match self.bits {
            8 => x as u8,
            b => ((x * 255 + ((1 << b) - 1) / 2) / ((1 << b) - 1)) as u8,
        }
    }
}

struct Masks {
    r: Channel,
    g: Channel,
    b: Channel,
    a: Option<Channel>,
}

impl Masks {
    fn new(r: u32, g: u32, b: u32, a: u32) -> Result<Self, BmpError> {
        let ch = |m| Channel::from_mask(m).ok_or(BmpError::Header("empty colour mask"));
        Ok(Self { r: ch(r)?, g: ch(g)?, b: ch(b)?, a: Channel::from_mask(a) })
    }

    fn px(&self, v: u32, out: &mut [u8]) {
        out[0] = self.r.get(v);
        out[1] = self.g.get(v);
        out[2] = self.b.get(v);
        out[3] = self.a.map_or(255, |a| a.get(v));
    }
}

/// Decode a BMP file to RGBA8, rows top-down.
pub fn decode(data: &[u8]) -> Result<Rgba, BmpError> {
    if data.len() < 18 || &data[0..2] != b"BM" {
        return Err(BmpError::Header("missing BM signature"));
    }
    let pixel_offset = u32le(data, 10)? as usize;
    let dib = u32le(data, 14)? as usize;
    let (width, height_raw, planes_at, bpp_at, compression, colors_used) = match dib {
        12 => (
            u16le(data, 18)? as i64,
            u16le(data, 20)? as i16 as i64,
            22,
            24,
            BI_RGB,
            0,
        ),
        40 | 52 | 56 | 64 | 108 | 124 => (
            u32le(data, 18)? as i32 as i64,
            u32le(data, 22)? as i32 as i64,
            26,
            28,
            u32le(data, 30)?,
            u32le(data, 46)? as usize,
        ),
        _ => return Err(BmpError::Header("unknown DIB header size")),
    };
    let _ = planes_at;
    let bpp = u16le(data, bpp_at)? as u32;
    let top_down = height_raw < 0;
    let height = height_raw.abs();
    if width <= 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return Err(BmpError::Dimensions(width, height_raw));
    }
    let (w, h) = (width as usize, height as usize);

    // Palette entries sit right after the header (and after the three mask dwords
    // of a bare 40-byte header with BI_BITFIELDS, which only 16/32-bit use).
    let palette_at = 14 + dib;
    let entry = if dib == 12 { 3 } else { 4 };
    let mut palette: Vec<[u8; 3]> = Vec::new();
    if bpp <= 8 {
        let n = if colors_used > 0 && colors_used <= (1 << bpp) { colors_used } else { 1usize << bpp };
        for i in 0..n {
            let at = palette_at + i * entry;
            let Some(e) = data.get(at..at + 3) else { break };
            palette.push([e[2], e[1], e[0]]);
        }
    }

    let mut out = vec![0u8; w * h * 4];
    let mut has_alpha = false;
    let rows = data.get(pixel_offset..).ok_or(BmpError::Truncated)?;
    let dst_row = |y: usize| if top_down { y } else { h - 1 - y };

    match (compression, bpp) {
        (BI_RLE8, 8) | (BI_RLE4, 4) => {
            let idx = rle_decode(rows, w, h, bpp == 4)?;
            for y in 0..h {
                let d = dst_row(y) * w * 4;
                for x in 0..w {
                    let c = palette.get(idx[y * w + x] as usize).copied().unwrap_or([0; 3]);
                    out[d + x * 4..d + x * 4 + 3].copy_from_slice(&c);
                    out[d + x * 4 + 3] = 255;
                }
            }
        }
        (BI_RGB, 1 | 4 | 8) => {
            let stride = (w * bpp as usize).div_ceil(32) * 4;
            for y in 0..h {
                let row = rows.get(y * stride..y * stride + (w * bpp as usize).div_ceil(8)).ok_or(BmpError::Truncated)?;
                let d = dst_row(y) * w * 4;
                for x in 0..w {
                    let i = match bpp {
                        8 => row[x],
                        4 => (row[x / 2] >> (if x % 2 == 0 { 4 } else { 0 })) & 0x0f,
                        _ => (row[x / 8] >> (7 - x % 8)) & 1,
                    };
                    let c = palette.get(i as usize).copied().unwrap_or([0; 3]);
                    out[d + x * 4..d + x * 4 + 3].copy_from_slice(&c);
                    out[d + x * 4 + 3] = 255;
                }
            }
        }
        (BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS, 16 | 24 | 32) => {
            let masks = if compression == BI_RGB || dib == 12 {
                match bpp {
                    16 => Masks::new(0x7c00, 0x03e0, 0x001f, 0)?,
                    _ => Masks::new(0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0)?,
                }
            } else {
                // Masks follow a 40-byte header, or are fields of the V3+ headers.
                let at = 54;
                let a = if compression == BI_ALPHABITFIELDS || dib >= 56 { u32le(data, at + 12).unwrap_or(0) } else { 0 };
                Masks::new(u32le(data, at)?, u32le(data, at + 4)?, u32le(data, at + 8)?, a)?
            };
            let bytes = bpp as usize / 8;
            let stride = (w * bpp as usize).div_ceil(32) * 4;
            for y in 0..h {
                let row = rows.get(y * stride..y * stride + w * bytes).ok_or(BmpError::Truncated)?;
                let d = dst_row(y) * w * 4;
                for x in 0..w {
                    let p = &row[x * bytes..x * bytes + bytes];
                    let v = match bytes {
                        2 => u16::from_le_bytes([p[0], p[1]]) as u32,
                        3 => u32::from_le_bytes([p[0], p[1], p[2], 0]),
                        _ => u32::from_le_bytes([p[0], p[1], p[2], p[3]]),
                    };
                    masks.px(v, &mut out[d + x * 4..d + x * 4 + 4]);
                }
            }
            has_alpha = masks.a.is_some();
        }
        (c, b) => return Err(BmpError::Unsupported(format!("{b}-bit, compression {c}"))),
    }
    Ok(Rgba { width: w as u32, height: h as u32, has_alpha, pixels: out })
}

/// Expand an RLE4/RLE8 stream to one palette index per pixel, rows bottom-up as
/// stored. Pixels the stream never writes stay index 0.
fn rle_decode(src: &[u8], w: usize, h: usize, four_bit: bool) -> Result<Vec<u8>, BmpError> {
    let mut idx = vec![0u8; w * h];
    // Stored row 0 is the bottom row, so this index space is bottom-up; the caller
    // maps stored row `y` to the output row.
    let (mut x, mut y) = (0usize, 0usize);
    let mut i = 0usize;
    let mut put = |x: usize, y: usize, v: u8| {
        if x < w && y < h {
            idx[y * w + x] = v;
        }
    };
    while i + 1 < src.len() {
        let (n, v) = (src[i] as usize, src[i + 1]);
        i += 2;
        if n > 0 {
            for k in 0..n {
                let val = if four_bit { if k % 2 == 0 { v >> 4 } else { v & 0x0f } } else { v };
                put(x, y, val);
                x += 1;
            }
            continue;
        }
        match v {
            0 => {
                x = 0;
                y += 1;
            }
            1 => break,
            2 => {
                let d = src.get(i..i + 2).ok_or(BmpError::Truncated)?;
                x += d[0] as usize;
                y += d[1] as usize;
                i += 2;
            }
            count => {
                let count = count as usize;
                let bytes = if four_bit { count.div_ceil(2) } else { count };
                let run = src.get(i..i + bytes).ok_or(BmpError::Truncated)?;
                for k in 0..count {
                    let val = if four_bit {
                        if k % 2 == 0 { run[k / 2] >> 4 } else { run[k / 2] & 0x0f }
                    } else {
                        run[k]
                    };
                    put(x, y, val);
                    x += 1;
                }
                i += bytes + (bytes & 1); // absolute runs pad to a 16-bit boundary
            }
        }
        if y >= h && x >= w {
            break;
        }
    }
    Ok(idx)
}

/// [`decode`] with the asset error type, for callers holding a path.
pub fn decode_bmp(path: &str, data: &[u8]) -> Result<Rgba, AssetError> {
    decode(data).map_err(|e| AssetError::Bmp(path.to_owned(), e))
}

/// Read and decode a `.bmp` through the VFS. `path` is a VFS path with the
/// extension, e.g. `bitmaps/title.bmp`; mod layers resolve as for every file.
pub fn load(vfs: &Vfs, path: &str) -> Result<Rgba, AssetError> {
    decode_bmp(path, &read(vfs, path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// File + 40-byte info header; `extra` is palette/masks, then `pixels`.
    fn bmp(w: i32, h: i32, bpp: u16, comp: u32, used: u32, extra: &[u8], pixels: &[u8]) -> Vec<u8> {
        let off = 14 + 40 + extra.len();
        let mut v = b"BM".to_vec();
        v.extend(((off + pixels.len()) as u32).to_le_bytes());
        v.extend([0; 4]);
        v.extend((off as u32).to_le_bytes());
        v.extend(40u32.to_le_bytes());
        v.extend(w.to_le_bytes());
        v.extend(h.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(bpp.to_le_bytes());
        v.extend(comp.to_le_bytes());
        v.extend((pixels.len() as u32).to_le_bytes());
        v.extend([0; 8]);
        v.extend(used.to_le_bytes());
        v.extend([0; 4]);
        v.extend(extra);
        v.extend(pixels);
        v
    }

    /// BGRX palette entries.
    fn pal(colors: &[[u8; 3]]) -> Vec<u8> {
        colors.iter().flat_map(|c| [c[2], c[1], c[0], 0]).collect()
    }

    const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 255, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    const WHITE: [u8; 3] = [255, 255, 255];

    fn px(img: &Rgba, x: usize, y: usize) -> [u8; 4] {
        let i = (y * img.width as usize + x) * 4;
        img.pixels[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn one_bit_palette_bottom_up() {
        // 9x2: stride 4 bytes. Stored row 0 is the bottom row.
        let p = pal(&[RED, WHITE]);
        let data = bmp(9, 2, 1, 0, 0, &p, &[0b1010_1010, 0b1000_0000, 0, 0, 0, 0, 0, 0]);
        let img = decode(&data).unwrap();
        assert_eq!((img.width, img.height, img.has_alpha), (9, 2, false));
        // Top row of the image is stored second: all index 0 = red.
        assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]);
        // Bottom row: 1,0,1,0,...,1 (bit 9 = 1).
        assert_eq!(px(&img, 0, 1), [255, 255, 255, 255]);
        assert_eq!(px(&img, 1, 1), [255, 0, 0, 255]);
        assert_eq!(px(&img, 8, 1), [255, 255, 255, 255]);
    }

    #[test]
    fn four_bit_palette_top_down() {
        let p = pal(&[RED, GREEN, BLUE, WHITE]);
        // Negative height = top-down. 3x2, stride 4.
        let data = bmp(3, -2, 4, 0, 4, &p, &[0x01, 0x20, 0, 0, 0x33, 0x00, 0, 0]);
        let img = decode(&data).unwrap();
        assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]);
        assert_eq!(px(&img, 1, 0), [0, 255, 0, 255]);
        assert_eq!(px(&img, 2, 0), [0, 0, 255, 255]);
        assert_eq!(px(&img, 0, 1), [255, 255, 255, 255]);
        assert_eq!(px(&img, 2, 1), [255, 0, 0, 255]);
    }

    #[test]
    fn eight_bit_palette_uses_clr_used() {
        let p = pal(&[RED, GREEN, BLUE]);
        let data = bmp(2, 1, 8, 0, 3, &p, &[2, 1, 0, 0]);
        let img = decode(&data).unwrap();
        assert_eq!(px(&img, 0, 0), [0, 0, 255, 255]);
        assert_eq!(px(&img, 1, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn rle8_runs_absolutes_and_delta() {
        let p = pal(&[RED, GREEN, BLUE]);
        // 4x3 bottom-up. Row0: run of 4 x idx1, EOL. Row1: absolute 3 (0,1,2)+pad, then
        // 1 x idx2, EOL. Row2: delta over 2 pixels, 2 x idx1, EOB.
        let rle = [
            4, 1, 0, 0, //
            0, 3, 0, 1, 2, 0, 1, 2, 0, 0, //
            0, 2, 2, 0, 2, 1, 0, 1,
        ];
        let data = bmp(4, 3, 8, BI_RLE8, 3, &p, &rle);
        let img = decode(&data).unwrap();
        // Output y = 2 is stored row 0.
        for x in 0..4 {
            assert_eq!(px(&img, x, 2), [0, 255, 0, 255]);
        }
        let row1: Vec<_> = (0..4).map(|x| px(&img, x, 1)[..3].to_vec()).collect();
        assert_eq!(row1, [RED, GREEN, BLUE, BLUE].map(|c| c.to_vec()));
        // Row 2 of the stream (output y = 0): first two untouched (index 0 = red).
        assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]);
        assert_eq!(px(&img, 2, 0), [0, 255, 0, 255]);
        assert_eq!(px(&img, 3, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn rle4_run_and_absolute() {
        let p = pal(&[RED, GREEN, BLUE, WHITE]);
        // 7x1: run of 3 alternating 1,2 -> 1,2,1; absolute of 4 (3,0,1,2); EOB.
        let rle = [3, 0x12, 0, 4, 0x30, 0x12, 0, 1];
        let data = bmp(7, 1, 4, BI_RLE4, 4, &p, &rle);
        let img = decode(&data).unwrap();
        let row: Vec<_> = (0..7).map(|x| px(&img, x, 0)[..3].to_vec()).collect();
        assert_eq!(row, [GREEN, BLUE, GREEN, WHITE, RED, GREEN, BLUE].map(|c| c.to_vec()));
    }

    #[test]
    fn truncated_rle_run_is_an_error() {
        let p = pal(&[RED, GREEN]);
        let data = bmp(8, 1, 8, BI_RLE8, 2, &p, &[0, 8, 1, 0]);
        assert!(matches!(decode(&data), Err(BmpError::Truncated)));
    }

    #[test]
    fn sixteen_bit_defaults_to_555() {
        // 0x7c00 = red, 0x03e0 = green, 0x001f = blue, 0x7fff = white
        let pixels: Vec<u8> = [0x7c00u16, 0x03e0, 0x001f, 0x7fff].iter().flat_map(|v| v.to_le_bytes()).collect();
        let data = bmp(4, 1, 16, 0, 0, &[], &pixels);
        let img = decode(&data).unwrap();
        assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]);
        assert_eq!(px(&img, 1, 0), [0, 255, 0, 255]);
        assert_eq!(px(&img, 2, 0), [0, 0, 255, 255]);
        assert_eq!(px(&img, 3, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn sixteen_bit_565_bitfields() {
        let mut masks = Vec::new();
        for m in [0xf800u32, 0x07e0, 0x001f] {
            masks.extend(m.to_le_bytes());
        }
        let pixels: Vec<u8> = [0xf800u16, 0x07e0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let data = bmp(2, 1, 16, BI_BITFIELDS, 0, &masks, &pixels);
        let img = decode(&data).unwrap();
        assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]);
        assert_eq!(px(&img, 1, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn twenty_four_bit_pads_rows() {
        // 1x2: stride 4 (3 bytes + 1 pad). BGR order. Bottom row stored first.
        let data = bmp(1, 2, 24, 0, 0, &[], &[0, 0, 255, 0, 0, 255, 0, 0]);
        let img = decode(&data).unwrap();
        assert_eq!(px(&img, 0, 1), [255, 0, 0, 255]); // bottom = red
        assert_eq!(px(&img, 0, 0), [0, 255, 0, 255]); // top = green
    }

    #[test]
    fn thirty_two_bit_plain_is_opaque_and_bitfields_carry_alpha() {
        let data = bmp(1, 1, 32, 0, 0, &[], &[10, 20, 30, 7]);
        let img = decode(&data).unwrap();
        assert_eq!(px(&img, 0, 0), [30, 20, 10, 255]);
        assert!(!img.has_alpha);

        let mut masks = Vec::new();
        for m in [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff, 0xff00_0000] {
            masks.extend(m.to_le_bytes());
        }
        let data = bmp(1, 1, 32, BI_ALPHABITFIELDS, 0, &masks, &[10, 20, 30, 7]);
        let img = decode(&data).unwrap();
        assert_eq!(px(&img, 0, 0), [30, 20, 10, 7]);
        assert!(img.has_alpha);
    }

    #[test]
    fn os2_core_header() {
        let mut v = b"BM".to_vec();
        v.extend(0u32.to_le_bytes());
        v.extend([0; 4]);
        v.extend((14u32 + 12 + 6).to_le_bytes());
        v.extend(12u32.to_le_bytes());
        v.extend(2u16.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(8u16.to_le_bytes());
        v.extend([0, 0, 255, 255, 0, 0]); // BGR triples: red, blue
        v.extend([1, 0, 0, 0]);
        let img = decode(&v).unwrap();
        assert_eq!(px(&img, 0, 0), [0, 0, 255, 255]);
        assert_eq!(px(&img, 1, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(decode(b"PNG not a bmp at all"), Err(BmpError::Header(_))));
        let data = bmp(0, 4, 8, 0, 0, &[], &[]);
        assert!(matches!(decode(&data), Err(BmpError::Dimensions(0, 4))));
        let data = bmp(100, 100, 24, 0, 0, &[], &[0; 10]);
        assert!(matches!(decode(&data), Err(BmpError::Truncated)));
        let data = bmp(1, 1, 24, 4, 0, &[], &[0; 4]);
        assert!(matches!(decode(&data), Err(BmpError::Unsupported(_))));
    }

    #[test]
    fn loads_through_the_vfs() {
        let dir = std::env::temp_dir().join(format!("sta_bmp_vfs_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("bitmaps")).unwrap();
        std::fs::write(dir.join("bitmaps/t.bmp"), bmp(1, 1, 24, 0, 0, &[], &[0, 0, 255, 0])).unwrap();
        let mut vfs = Vfs::default();
        vfs.push_dir("t".into(), dir.clone()).unwrap();
        let img = load(&vfs, "bitmaps/t.bmp").unwrap();
        assert_eq!(px(&img, 0, 0), [255, 0, 0, 255]);
        assert!(matches!(load(&vfs, "bitmaps/none.bmp"), Err(AssetError::NotFound(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bmp")) {
                out.push(p);
            }
        }
    }

    /// Install-gated: `STA64_TEST_BMP_DIRS` is a path list (like `PATH`) of install roots;
    /// every `.bmp` below them must decode.
    #[test]
    fn every_installed_bmp_decodes() {
        let Some(dirs) = std::env::var_os("STA64_TEST_BMP_DIRS") else {
            eprintln!("SKIP: set STA64_TEST_BMP_DIRS to a path list of A1/A2/FO install roots");
            return;
        };
        let mut files = Vec::new();
        for d in std::env::split_paths(&dirs) {
            walk(&d, &mut files);
        }
        let mut failures = Vec::new();
        for f in &files {
            let data = std::fs::read(f).unwrap();
            if let Err(e) = decode(&data) {
                failures.push(format!("{}: {e}", f.display()));
            }
        }
        eprintln!("decoded {} of {} BMPs", files.len() - failures.len(), files.len());
        assert!(failures.is_empty(), "{} failed:\n{}", failures.len(), failures.join("\n"));
    }
}
