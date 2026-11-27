//! Probe of the listing against GNU objdump 2.47 on bytes this test authors itself (a seeded
//! pseudo-random blob and systematic encodings; never game data). Needs objdump 2.47 and skips
//! otherwise. Lengths and text are compared separately: a length difference moves every
//! instruction after it, so those episodes must be zero; text-only differences are counted by
//! class and printed (set `STA64_PROBE_VERBOSE=<n>` to see up to n episodes with their bytes).

use std::collections::BTreeMap;
use std::process::Command;

use sta_mission_convert::disasm::{disassemble_bytes, parse_objdump};

const BASE: u32 = 0x1000_0000;

fn objdump_ok() -> bool {
    match Command::new("objdump").arg("--version").output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .next()
            .is_some_and(|l| l.contains(" 2.47")),
        Err(_) => false,
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

fn random_blob(seed: u64, len: usize) -> Vec<u8> {
    let mut r = Rng(seed);
    let mut v = Vec::with_capacity(len);
    while v.len() < len {
        let x = r.next().to_le_bytes();
        v.extend_from_slice(&x);
    }
    v.truncate(len);
    v
}

/// Every opcode with a few ModRM bytes in the one-byte, `0f`, `0f 38` and `0f 3a` maps, and
/// seven prefixes in front, each in a 32-byte slot padded with `nop`.
fn systematic_slots() -> Vec<u8> {
    let mut out = Vec::new();
    let mut add = |b: &[u8]| {
        let mut s = b.to_vec();
        s.resize(32, 0x90);
        out.extend_from_slice(&s);
    };
    let mods = [
        0x00u8, 0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38, 0xc0, 0xc8, 0xd0, 0xd8, 0xe0, 0xe8, 0xf0,
        0xf8,
    ];
    for op in 0..=255u8 {
        if op != 0x0f {
            for m in mods {
                add(&[op, m]);
            }
        }
    }
    for op in 0..=255u8 {
        for m in mods {
            add(&[0x0f, op, m]);
        }
    }
    for op in 0..=255u8 {
        for m in [0x00u8, 0xc0] {
            add(&[0x0f, 0x38, op, m]);
            add(&[0x0f, 0x3a, op, m, 0x01]);
        }
    }
    for p in [0x66u8, 0x67, 0xf0, 0xf2, 0xf3, 0x2e, 0x64] {
        for op in 0..=255u8 {
            if [
                0x0f, 0x66, 0x67, 0xf0, 0xf2, 0xf3, 0x2e, 0x64, 0x36, 0x3e, 0x26, 0x65,
            ]
            .contains(&op)
            {
                continue;
            }
            for m in [0x00u8, 0xc0] {
                add(&[p, op, m]);
            }
        }
        for op in 0..=255u8 {
            for m in [0x00u8, 0xc0] {
                add(&[p, 0x0f, op, m]);
            }
        }
    }
    // Longer fixed forms: x87 with prefixes, fwait runs, long prefix runs.
    for op in 0xd8..=0xdfu8 {
        for m in [0x00u8, 0x45, 0xc1, 0xe9, 0xf1] {
            for pre in [
                &[][..],
                &[0x9b],
                &[0x9b, 0x9b],
                &[0x66],
                &[0x9b, 0x66],
                &[0x66, 0x9b],
                &[0x9b, 0xf0],
                &[0x9b, 0x9b, 0x9b],
            ] {
                let mut b = pre.to_vec();
                b.extend_from_slice(&[op, m, 0x90]);
                add(&b);
            }
        }
    }
    for n in 1..=17usize {
        for p in [0x66u8, 0x3e, 0xf0, 0xf2, 0x67] {
            let mut b = vec![p; n];
            b.extend_from_slice(&[0x8b, 0x00]);
            add(&b);
            let mut b = vec![p; n];
            b.push(0x90);
            add(&b);
        }
    }
    for n in 0..=10usize {
        let mut b = vec![0x3e; n];
        b.extend_from_slice(&[0x81, 0x84, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        add(&b);
    }
    out
}

struct Outcome {
    instructions: usize,
    length_episodes: usize,
    text_diffs: usize,
    classes: BTreeMap<String, usize>,
}

fn compare(name: &str, blob: &[u8], dir: &std::path::Path) -> Outcome {
    let path = dir.join(format!("probe-{name}.bin"));
    std::fs::write(&path, blob).expect("probe file written");
    let out = Command::new("objdump")
        .args([
            "-D",
            "-b",
            "binary",
            "-m",
            "i386",
            "-M",
            "intel",
            "--no-show-raw-insn",
        ])
        .arg(format!("--adjust-vma={BASE:#x}"))
        .arg(&path)
        .output()
        .expect("objdump runs");
    let want = parse_objdump(&String::from_utf8_lossy(&out.stdout));
    let mut got = Vec::new();
    disassemble_bytes(BASE, blob, &mut got);
    let verbose: usize = std::env::var("STA64_PROBE_VERBOSE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let hex_at = |a: u32| -> String {
        let o = (a - BASE) as usize;
        blob[o..blob.len().min(o + 12)]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut o = Outcome {
        instructions: want.len(),
        length_episodes: 0,
        text_diffs: 0,
        classes: BTreeMap::new(),
    };
    let (mut i, mut j) = (0, 0);
    let mut in_episode = false;
    let mut last: Option<(u32, String)> = None;
    let mut shown = 0;
    while i < want.len() && j < got.len() {
        let (a, b) = (&want[i], &got[j]);
        if a.0 == b.0 {
            in_episode = false;
            last = Some((a.0, a.1.clone()));
            if a.1 != b.1 {
                o.text_diffs += 1;
                let w = a.1.split(' ').next().unwrap_or("").to_string();
                let g = b.1.split(' ').next().unwrap_or("").to_string();
                *o.classes.entry(format!("{w} -> {g}")).or_default() += 1;
                if shown < verbose {
                    shown += 1;
                    eprintln!(
                        "[{name}] text   {}: want {:?} got {:?}",
                        hex_at(a.0),
                        a.1,
                        b.1
                    );
                }
            }
            i += 1;
            j += 1;
            continue;
        }
        if !in_episode {
            in_episode = true;
            o.length_episodes += 1;
            if shown < verbose {
                shown += 1;
                // The instruction before the split is the one whose length differs.
                let (la, lt) = last.clone().unwrap_or((a.0, String::new()));
                eprintln!(
                    "[{name}] LENGTH at {}: before {:?}; objdump next {:#x} {:?}, listing next {:#x} {:?}",
                    hex_at(la),
                    lt,
                    a.0 - la,
                    a.1,
                    b.0 - la,
                    b.1
                );
            }
        }
        if a.0 < b.0 {
            i += 1;
        } else {
            j += 1;
        }
    }
    if (i < want.len()) != (j < got.len()) {
        o.length_episodes += 1;
    }
    o
}

#[test]
fn listing_lengths_equal_objdump_on_random_and_systematic_bytes() {
    if !objdump_ok() {
        eprintln!("objdump 2.47 not available: skipped");
        return;
    }
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    // Chunks of one megabyte at most, compared and dropped one after another.
    const CHUNK: usize = 1 << 20;
    let mut length_total = 0;
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    let (mut instructions, mut text_total) = (0usize, 0usize);
    let verbose = std::env::var("STA64_PROBE_VERBOSE").is_ok();
    let mut run = |name: &str, blob: &[u8]| {
        let o = compare(name, blob, &dir);
        instructions += o.instructions;
        text_total += o.text_diffs;
        length_total += o.length_episodes;
        for (k, c) in o.classes {
            *classes.entry(k).or_default() += c;
        }
    };
    for (i, c) in systematic_slots().chunks(CHUNK).enumerate() {
        run(&format!("systematic-{i}"), c);
    }
    for seed in [1288u64, 77] {
        for i in 0..4u64 {
            run(
                &format!("random-{seed}-{i}"),
                &random_blob(seed * 1000 + i, CHUNK),
            );
        }
    }
    eprintln!(
        "{instructions} instructions, {length_total} length episodes, {text_total} text-only differences"
    );
    if verbose {
        let mut v: Vec<_> = classes.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        for (k, c) in v.iter().take(60) {
            eprintln!("    {c:6} {k}");
        }
    }
    assert_eq!(
        length_total, 0,
        "the listing's instruction lengths differ from objdump's"
    );
}
