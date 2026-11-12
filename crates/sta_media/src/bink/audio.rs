//! Bink audio: blocks of band-quantised transform coefficients (a DCT or a
//! real inverse DFT), overlapped by a short linear crossfade. Follows the
//! MIT `mercs2_bink` clean-room description (`SPEC-audio.md`); the inverse
//! transforms are computed here with a plain radix-2 FFT.

use super::bits::Bits;
use super::tables::AUDIO_QUANT_BITS;

/// Track flag: the DCT transform (`binkaudio_dct`).
pub(crate) const TRACK_DCT: u32 = 0x1000_0000;
/// Track flag: two channels.
pub(crate) const TRACK_STEREO: u32 = 0x2000_0000;
/// Track flag: 16-bit output (else 8-bit).
pub(crate) const TRACK_16BIT: u32 = 0x4000_0000;
/// Track flag: the real-DFT transform (`binkaudio_rdft`); wins over DCT.
pub(crate) const TRACK_RDFT: u32 = 0x8000_0000;

/// Critical-band edges in Hz.
const FREQ: [u32; 25] = [
    0, 100, 200, 300, 400, 510, 630, 770, 920, 1080, 1270, 1480, 1720, 2000, 2320, 2700, 3150,
    3700, 4400, 5300, 6400, 7700, 9500, 12000, 15500,
];

/// Coefficient run lengths, in units of 8.
const RUN_LEN: [usize; 16] = [2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 32, 64];

/// How a track's flags describe it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TrackFormat {
    pub rate: u32,
    pub channels: u16,
    pub dct: bool,
    pub sixteen_bit: bool,
}

impl TrackFormat {
    /// `None` when the track holds no decodable audio.
    pub fn from_flags(flags: u32) -> Option<Self> {
        if flags & (TRACK_DCT | TRACK_RDFT) == 0 {
            return None;
        }
        Some(Self {
            rate: flags & 0xffff,
            channels: if flags & TRACK_STEREO != 0 { 2 } else { 1 },
            dct: flags & TRACK_RDFT == 0,
            sixteen_bit: flags & TRACK_16BIT != 0,
        })
    }

    /// Bytes one sample takes in the container's decoded-size field: the
    /// field counts 16-bit samples even for 8-bit tracks (measured on the
    /// Armada installs, whose 8-bit tracks state exactly twice their
    /// samples).
    pub fn bytes_per_sample(&self) -> u32 {
        2
    }
}

/// In-place iterative radix-2 complex FFT with a `+` exponent
/// (`Z_k = sum z_j e^{+2 pi i jk/n}`), unnormalised.
struct Fft {
    n: usize,
    twiddle: Vec<(f64, f64)>,
    rev: Vec<usize>,
}

impl Fft {
    fn new(n: usize) -> Self {
        assert!(n.is_power_of_two());
        let bits = n.trailing_zeros();
        let rev = (0..n)
            .map(|i| {
                if bits == 0 {
                    0
                } else {
                    i.reverse_bits() >> (usize::BITS - bits)
                }
            })
            .collect();
        let twiddle = (0..n / 2)
            .map(|k| {
                let a = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
                (a.cos(), a.sin())
            })
            .collect();
        Self { n, twiddle, rev }
    }

    fn run(&self, re: &mut [f64], im: &mut [f64]) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = self.twiddle[k * step];
                    let (a, b) = (start + k, start + k + len / 2);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }
    }
}

/// Decoder state of one audio track.
pub(crate) struct AudioDecoder {
    pub format: TrackFormat,
    /// Transform size.
    n: usize,
    /// Channels coded one after the other in a block (1 for RDFT, which
    /// carries interleaved samples in a single transform).
    ch_loop: usize,
    bands: Vec<usize>,
    scale: f64,
    /// Interleaved int16 values one block produces before overlap removal.
    total: usize,
    overlap: usize,
    prev: Vec<i32>,
    first: bool,
    fft: Fft,
    /// DCT pre-twist `e^{i pi j / 2n}`.
    twist: Vec<(f64, f64)>,
    quant: [f32; 96],
}

impl AudioDecoder {
    pub fn new(format: TrackFormat) -> Self {
        let rate = format.rate;
        let channels = format.channels as usize;
        let base = if rate >= 44100 {
            2048
        } else if rate >= 22050 {
            1024
        } else {
            512
        };
        let total = base * channels;
        let (n, ch_loop, r) = if format.dct {
            (base, channels, rate)
        } else {
            (total, 1, rate * channels as u32)
        };
        let half = n >> 1;
        let hr = r.div_ceil(2).max(1);
        let nb = FREQ.iter().position(|&f| hr <= f).unwrap_or(FREQ.len());
        let mut bands: Vec<usize> = (0..nb)
            .map(|i| ((FREQ[i] as u64 * half as u64) / hr as u64).max(1) as usize)
            .collect();
        bands.push(half);
        let fft = Fft::new(if format.dct { 2 * n } else { n });
        let twist = (0..n)
            .map(|j| {
                let a = std::f64::consts::PI * j as f64 / (2 * n) as f64;
                (a.cos(), a.sin())
            })
            .collect();
        let mut quant = [0f32; 96];
        for (q, bits) in quant.iter_mut().zip(AUDIO_QUANT_BITS.iter()) {
            *q = f32::from_bits(*bits);
        }
        Self {
            format,
            n,
            ch_loop,
            bands,
            scale: (2.0f32 / (n as f32).sqrt()) as f64,
            total,
            overlap: total / 16,
            prev: vec![0; total / 16],
            first: true,
            fft,
            twist,
            quant,
        }
    }

    /// Forget the overlap history (after a seek).
    pub fn reset(&mut self) {
        self.first = true;
        self.prev.fill(0);
    }

    /// Interleaved samples one block yields.
    #[cfg(test)]
    pub fn block_samples(&self) -> usize {
        self.total - self.overlap
    }

    /// Decode one frame's record of this track (the bytes after the record
    /// length: decoded size, then blocks) into interleaved int16 samples.
    pub fn decode_record(&mut self, record: &[u8]) -> Result<Vec<i16>, String> {
        if record.len() < 4 {
            return Err("audio record too short".into());
        }
        let out_bytes = u32::from_le_bytes(record[..4].try_into().unwrap());
        let mut remaining = (out_bytes / self.format.bytes_per_sample()) as usize;
        // Every block takes at least one 32-bit word: cap the reservation by
        // what the record can actually yield, not by the declared size.
        let most = (record.len() / 4).saturating_mul(self.total);
        let mut out = Vec::with_capacity(remaining.min(most));
        let mut pos = 4;
        while remaining != 0 {
            if pos >= record.len() {
                return Err(format!("audio record ends with {remaining} samples to go"));
            }
            let (samples, used) = self.decode_block(&record[pos..])?;
            let take = samples.len().min(remaining);
            out.extend_from_slice(&samples[..take]);
            remaining -= take;
            pos += used;
        }
        // The declared size must account for every coded block.
        if record.len().saturating_sub(pos) >= 4 {
            return Err(format!(
                "audio record declares {out_bytes} bytes but leaves {} bytes of blocks undecoded",
                record.len() - pos
            ));
        }
        Ok(out)
    }

    fn decode_block(&mut self, data: &[u8]) -> Result<(Vec<i16>, usize), String> {
        let mut b = Bits::new(data);
        let n = self.n;
        if self.format.dct {
            b.skip(2);
        }
        let mut pcm = vec![0f64; n * self.ch_loop];
        let mut coeffs = vec![0f32; n];
        let nb = self.bands.len() - 1;
        let mut quant = vec![0f32; nb];
        for ch in 0..self.ch_loop {
            coeffs[0] = read_float29(&mut b)?;
            coeffs[1] = read_float29(&mut b)?;
            for q in quant.iter_mut() {
                *q = self.quant[(b.read(8) as usize).min(95)];
            }
            let mut i = 2usize;
            let mut k = 0usize;
            let mut q = 0f32;
            while i < n {
                let end = if b.bit() {
                    i + 8 * RUN_LEN[b.read(4) as usize]
                } else {
                    i + 8
                };
                let end = end.min(n);
                let width = b.read(4);
                if width == 0 {
                    coeffs[i..end].fill(0.0);
                    i = end;
                    while k < nb && 2 * self.bands[k] < end {
                        q = quant[k];
                        k += 1;
                    }
                } else {
                    while i < end {
                        if k < nb && i == 2 * self.bands[k] {
                            q = quant[k];
                            k += 1;
                        }
                        let v = b.read(width);
                        coeffs[i] = if v == 0 {
                            0.0
                        } else if b.bit() {
                            -q * v as f32
                        } else {
                            q * v as f32
                        };
                        i += 1;
                    }
                }
            }
            if b.overrun() {
                return Err("audio block ends early".into());
            }
            let out = &mut pcm[ch * n..(ch + 1) * n];
            if self.format.dct {
                self.inverse_dct(&coeffs, out);
            } else {
                self.inverse_rdft(&coeffs, out);
            }
        }
        // To integers, interleaving the separately coded channels. Values are
        // clipped to 16 bits only after the crossfade, as ffmpeg does; the
        // 2007 binkw32 described by the mercs2_bink specification clips
        // before it, which differs only on clipping passages (decision
        // recorded in the MED-01 PR).
        let mut block = vec![0i32; self.total];
        if self.ch_loop == 1 {
            for (o, x) in block.iter_mut().zip(pcm.iter()) {
                *o = to_int(*x * self.scale);
            }
        } else {
            for i in 0..n {
                for ch in 0..self.ch_loop {
                    block[i * self.ch_loop + ch] = to_int(pcm[ch * n + i] * self.scale);
                }
            }
        }
        let m = self.overlap;
        if self.first {
            self.first = false;
        } else {
            for (i, (out, prev)) in block[..m].iter_mut().zip(&self.prev).enumerate() {
                let s = *prev as i64 * (m - i) as i64 + *out as i64 * i as i64;
                *out = s.div_euclid(m as i64) as i32;
            }
        }
        self.prev.copy_from_slice(&block[self.total - m..]);
        block.truncate(self.total - m);
        let block = block
            .iter()
            .map(|&v| v.clamp(-32768, 32767) as i16)
            .collect();
        Ok((block, b.words_used_bytes()))
    }

    /// `x[k] = sum_j c[j] cos(pi j (k + 1/2) / n)`.
    fn inverse_dct(&self, c: &[f32], out: &mut [f64]) {
        let n = self.n;
        let mut re = vec![0f64; 2 * n];
        let mut im = vec![0f64; 2 * n];
        for j in 0..n {
            let (tc, ts) = self.twist[j];
            re[j] = c[j] as f64 * tc;
            im[j] = c[j] as f64 * ts;
        }
        self.fft.run(&mut re, &mut im);
        out.copy_from_slice(&re[..n]);
    }

    /// `x[k] = (c0 + c1 (-1)^k) / 2 + sum_{j=1}^{n/2-1} c[2j] cos(2 pi jk/n) + c[2j+1] sin(2 pi jk/n)`.
    fn inverse_rdft(&self, c: &[f32], out: &mut [f64]) {
        let n = self.n;
        let mut re = vec![0f64; n];
        let mut im = vec![0f64; n];
        re[0] = c[0] as f64 * 0.5;
        re[n / 2] = c[1] as f64 * 0.5;
        for j in 1..n / 2 {
            re[j] = c[2 * j] as f64;
            im[j] = -(c[2 * j + 1] as f64);
        }
        self.fft.run(&mut re, &mut im);
        out.copy_from_slice(&re);
    }
}

fn read_float29(b: &mut Bits) -> Result<f32, String> {
    let v = b.read(29);
    let e = (v & 31) as i32;
    let mant = (v >> 5) & 0x7f_ffff;
    let f = mant as f32 * 2f32.powi(e - 23);
    Ok(if v & 0x1000_0000 != 0 { -f } else { f })
}

/// Round half to even (saturating far outside the 16-bit range).
fn to_int(x: f64) -> i32 {
    x.round_ties_even().clamp(-1e9, 1e9) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_layout_matches_the_spec() {
        let d = AudioDecoder::new(TrackFormat {
            rate: 48000,
            channels: 2,
            dct: true,
            sixteen_bit: true,
        });
        assert_eq!(d.n, 2048);
        assert_eq!(
            d.bands,
            [
                1, 4, 8, 12, 17, 21, 26, 32, 39, 46, 54, 63, 73, 85, 98, 115, 134, 157, 187, 226,
                273, 328, 405, 512, 661, 1024
            ]
        );
        assert_eq!(d.block_samples(), 3840);
        let d = AudioDecoder::new(TrackFormat {
            rate: 22050,
            channels: 1,
            dct: true,
            sixteen_bit: true,
        });
        assert_eq!(d.bands.len(), 24);
        assert_eq!(d.block_samples(), 960);
        assert!((d.scale - 0.0625).abs() < 1e-9);
    }

    #[test]
    fn transforms_match_their_closed_forms() {
        for dct in [true, false] {
            let d = AudioDecoder::new(TrackFormat {
                rate: 22050,
                channels: 1,
                dct,
                sixteen_bit: true,
            });
            let n = d.n;
            let c: Vec<f32> = (0..n)
                .map(|i| ((i * 37 % 11) as f32 - 5.0) * 0.25)
                .collect();
            let mut fast = vec![0f64; n];
            if dct {
                d.inverse_dct(&c, &mut fast);
            } else {
                d.inverse_rdft(&c, &mut fast);
            }
            for k in [0usize, 1, 7, n / 2, n - 1] {
                let mut slow = 0f64;
                if dct {
                    for (j, cj) in c.iter().enumerate() {
                        slow += *cj as f64
                            * (std::f64::consts::PI * j as f64 * (k as f64 + 0.5) / n as f64).cos();
                    }
                } else {
                    slow = 0.5 * (c[0] as f64 + c[1] as f64 * if k % 2 == 0 { 1.0 } else { -1.0 });
                    for j in 1..n / 2 {
                        let a = 2.0 * std::f64::consts::PI * (j * k) as f64 / n as f64;
                        slow += c[2 * j] as f64 * a.cos() + c[2 * j + 1] as f64 * a.sin();
                    }
                }
                assert!(
                    (slow - fast[k]).abs() < 1e-6,
                    "dct={dct} k={k}: {slow} vs {}",
                    fast[k]
                );
            }
        }
    }
}
