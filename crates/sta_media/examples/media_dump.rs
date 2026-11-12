//! Decode a movie and report (or dump) what comes out.
//!
//! `cargo run -p sta_media --release --example media_dump -- <movie> [--yuv out.yuv] [--pcm prefix] [--frames N]`
//!
//! `--yuv` writes the frames as raw `yuv420p` (Y, U, V per frame, cropped),
//! comparable with `ffmpeg -i <movie> -f rawvideo -pix_fmt yuv420p`.
//! `--yuv -` writes them to stdout (the report then goes to stderr).
//! `--pcm` writes each audio track as `<prefix>.<track>.s16le`.

use sta_media::{MediaEvent, Movie, Picture};
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: media_dump <movie> [--yuv file] [--pcm prefix] [--frames N]")?;
    let (mut yuv_path, mut pcm_prefix, mut max_frames) = (None, None, u64::MAX);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--yuv" => yuv_path = args.next(),
            "--pcm" => pcm_prefix = args.next(),
            "--frames" => max_frames = args.next().ok_or("--frames N")?.parse()?,
            _ => return Err(format!("unknown argument {a}").into()),
        }
    }
    let mut movie = Movie::open_bytes(std::fs::read(&path)?)?;
    let info = movie.info().clone();
    let mut yuv: Option<Box<dyn Write>> = match yuv_path.as_deref() {
        None => None,
        Some("-") => Some(Box::new(std::io::BufWriter::new(std::io::stdout().lock()))),
        Some(p) => Some(Box::new(std::io::BufWriter::new(std::fs::File::create(p)?))),
    };
    let report_to_stderr = yuv_path.as_deref() == Some("-");
    let mut pcm: Vec<Option<std::io::BufWriter<std::fs::File>>> = Vec::new();
    for t in 0..info.audio.len() {
        pcm.push(match &pcm_prefix {
            Some(p) => Some(std::io::BufWriter::new(std::fs::File::create(format!(
                "{p}.{t}.s16le"
            ))?)),
            None => None,
        });
    }
    let mut frames = 0u64;
    let mut samples = vec![0u64; info.audio.len()];
    while let Some(event) = movie.next_event()? {
        match event {
            MediaEvent::Video(f) => {
                if frames >= max_frames {
                    break;
                }
                frames += 1;
                if let (Some(out), Picture::Yuv420 { y, u, v, .. }) = (&mut yuv, &f.picture) {
                    for p in [y, u, v] {
                        for r in 0..p.height {
                            out.write_all(&p.row(r)[..p.width as usize])?;
                        }
                    }
                }
            }
            MediaEvent::Audio(c) => {
                samples[c.track] += c.frames() as u64;
                if let Some(out) = &mut pcm[c.track] {
                    for s in &c.samples {
                        out.write_all(
                            &((s * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes(),
                        )?;
                    }
                }
            }
        }
    }
    drop(yuv);
    let mut report = String::new();
    let v = info.video.as_ref();
    use std::fmt::Write as _;
    writeln!(
        report,
        "{path}\tframes {frames}/{}\t{}",
        v.and_then(|v| v.frame_count)
            .map_or("?".into(), |n| n.to_string()),
        v.map_or(String::new(), |v| format!(
            "{}x{} {}",
            v.width, v.height, v.codec
        )),
    )?;
    for (t, a) in info.audio.iter().enumerate() {
        writeln!(
            report,
            "\ttrack {t}: {} Hz x{} {}\tsamples {}/{}",
            a.sample_rate,
            a.channels,
            a.codec,
            samples[t],
            a.total_frames.map_or("?".into(), |n| n.to_string())
        )?;
    }
    if report_to_stderr {
        eprint!("{report}");
    } else {
        print!("{report}");
    }
    Ok(())
}
