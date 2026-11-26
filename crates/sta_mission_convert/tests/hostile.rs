//! Edges that matter for parity (sections, padding, overlap), the map matcher on non-ASCII text,
//! the `Image` trait's import and export-size accessors, and STA64's own limits on crafted
//! tables. Files are built by the test from its own bytes and patched in place.

use sta_mission_convert::fixture::{CODE, DATA, Import, PeBuilder};
use sta_mission_convert::map::{MapFile, engine_vtables_from};
use sta_mission_convert::{FlatImage, Image, Pe};

const BASE: u32 = 0x1000_0000;
/// Where the section table starts in a built file.
const HEADERS: usize = 0x40 + 24 + 224;

fn get32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// Offset of the named section's header.
fn header(b: &[u8], name: &str) -> usize {
    (0..17)
        .map(|i| HEADERS + 40 * i)
        .find(|&h| b[h..h + 8].starts_with(name.as_bytes()) && b[h + name.len()] == 0)
        .expect("section")
}

// ------------------------------------------------------------------ sections

#[test]
fn section_of_stops_at_the_virtual_size_where_off_goes_on_into_the_raw_padding() {
    let pe = Pe::from_bytes(
        PeBuilder::new(BASE)
            .section(".rdata", 0x2000, &[7; 0x20], DATA)
            .build()
            .bytes,
    )
    .unwrap();
    assert!(pe.off(0x2040).is_some());
    assert_eq!(pe.section_of(BASE + 0x201F), Some(".rdata"));
    assert_eq!(pe.section_of(BASE + 0x2020), None);
}

#[test]
fn a_section_larger_in_memory_than_on_disk_maps_nothing_in_its_tail() {
    // Raw data 0x200 (padded), virtual size 0x400.
    let pe = Pe::from_bytes(
        PeBuilder::new(BASE)
            .section_sized(".mix", 0x5000, 0x400, &[1; 0x80], DATA)
            .build()
            .bytes,
    )
    .unwrap();
    assert!(pe.off(0x5000 + 0x1FF).is_some());
    assert_eq!(pe.off(0x5000 + 0x200), None);
    assert_eq!(pe.section_of(BASE + 0x5000 + 0x3FF), Some(".mix"));
    assert_eq!(pe.section_of(BASE + 0x5000 + 0x400), None);
}

#[test]
fn overlapping_sections_the_first_wins_in_off_and_in_section_of() {
    let pe = Pe::from_bytes(
        PeBuilder::new(BASE)
            .section(".a", 0x1000, &[0xAA; 0x100], DATA)
            .section(".b", 0x1080, &[0xBB; 0x100], DATA)
            .build()
            .bytes,
    )
    .unwrap();
    assert_eq!(pe.section_of(BASE + 0x1090), Some(".a"));
    assert_eq!(pe.section_of(BASE + 0x1100), Some(".b"));
    // The second section's byte at 0x1100 is 0xBB; the first section's raw padding answers.
    assert_eq!(pe.data[pe.off(0x1100).unwrap()], 0);
    assert_eq!(pe.data[pe.off(0x1090).unwrap()], 0xAA);
}

// ------------------------------------------------------------------ the image trait

fn import_count<I: Image>(i: &I) -> usize {
    i.imports().len()
}

#[test]
fn the_trait_carries_imports_and_the_export_size() {
    let built = PeBuilder::new(BASE)
        .section(".text", 0x1000, &[0x90; 8], CODE)
        .import("one.dll", vec![Import::name("alpha"), Import::Ordinal(2)])
        .export("x", 0x1000)
        .build();
    let pe = Pe::from_bytes(built.bytes).unwrap();
    assert_eq!(import_count(&pe), 2);
    assert!(pe.export_size() > 0);

    let flat = FlatImage::new(BASE, 0x100)
        .with_import(BASE + 0x40, "fake.dll", "beta")
        .with_export_size(0x30);
    assert_eq!(import_count(&flat), 1);
    assert_eq!(
        flat.imports()[&(BASE + 0x40)],
        ("fake.dll".into(), "beta".into())
    );
    assert_eq!(flat.export_size(), 0x30);
    assert_eq!(FlatImage::new(BASE, 8).export_size(), 0);
}

// ------------------------------------------------------------------ the map on non-ASCII text

#[test]
fn a_non_ascii_character_inside_the_first_13_bytes_is_no_match_and_no_panic() {
    // A two-byte character at byte 3 (then 8 ASCII bytes) and at byte 4 (then 7).
    let text = "abc\u{e9}00000000 x 00401000 f\nabcd\u{e9}0000000 y 00401004 f\n\
                \u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9} z 00401008 f\n 0001:00000000 ok 0040100c f\n";
    let map = MapFile::parse(text);
    let got: Vec<(u32, &str)> = map.names.iter().map(|(&a, s)| (a, s.as_str())).collect();
    assert_eq!(got, [(0x40100c, "ok")]);
}

// ------------------------------------------------------------------ the limits

#[test]
fn a_crafted_import_directory_is_refused() {
    // One DLL with many symbols, then many DLLs whose lookup tables all point at its table.
    let big: Vec<Import> = (0..2000).map(|_| Import::name("s")).collect();
    let mut b = PeBuilder::new(BASE)
        .section(".text", 0x1000, &[0x90; 8], CODE)
        .import("big.dll", big);
    for _ in 0..200 {
        b = b.import("e.dll", Vec::new());
    }
    let mut bytes = b.build().bytes;
    // Unpatched, the file is fine.
    let ok = Pe::from_bytes(bytes.clone()).unwrap();
    assert_eq!(ok.imports.len(), 2000);

    let h = header(&bytes, ".idata");
    let (va, raw) = (get32(&bytes, h + 12), get32(&bytes, h + 20) as usize);
    let ilt0 = get32(&bytes, raw);
    assert!(ilt0 > va);
    for k in 1..=200 {
        put32(&mut bytes, raw + 20 * k, ilt0);
    }
    let err = Pe::from_bytes(bytes).unwrap_err();
    assert_eq!(err.message, "import table too large");
    assert!(err.raw);
}

#[test]
fn crafted_export_names_that_overlap_are_refused() {
    let mut b = PeBuilder::new(BASE)
        .section(".text", 0x1000, &[0x90; 8], CODE)
        .export(&"y".repeat(3000), 0x1000);
    for i in 1..400 {
        b = b.export(&format!("n{i}"), 0x1000);
    }
    let mut bytes = b.build().bytes;
    let ok = Pe::from_bytes(bytes.clone()).unwrap();
    assert_eq!(ok.exports().unwrap().len(), 400);

    let h = header(&bytes, ".edata");
    let (va, raw) = (get32(&bytes, h + 12), get32(&bytes, h + 20) as usize);
    let enpt = raw + (get32(&bytes, raw + 32) - va) as usize;
    let first = get32(&bytes, enpt);
    for i in 1..400 {
        put32(&mut bytes, enpt + 4 * i, first + i as u32);
    }
    let err = Pe::from_bytes(bytes).unwrap().exports().unwrap_err();
    assert_eq!(err.message, "export table too large");
    assert!(err.raw);
}

#[test]
fn more_export_names_than_words_in_the_file_is_refused() {
    let mut bytes = PeBuilder::new(BASE)
        .section(".text", 0x1000, &[0x90; 8], CODE)
        .export("a", 0x1000)
        .build()
        .bytes;
    let h = header(&bytes, ".edata");
    let raw = get32(&bytes, h + 20) as usize;
    put32(&mut bytes, raw + 24, u32::MAX);
    let err = Pe::from_bytes(bytes).unwrap().exports().unwrap_err();
    assert_eq!(err.message, "export table too large");
}

#[test]
fn a_vtable_walk_over_aliased_sections_is_bounded_by_the_file() {
    // Fifteen sections at consecutive addresses, each full of code pointers, all pointing at
    // the same raw bytes: the walk could run for 15 * 128 slots in a file of 512 words.
    let words: Vec<u8> = (0..128).flat_map(|_| 0x40_1010u32.to_le_bytes()).collect();
    let mut b = PeBuilder::new(0x40_0000).section(".text", 0x1000, &[0x90; 0x100], CODE);
    for n in 0..15u32 {
        b = b.section(&format!(".rd{n}"), 0x2000 + 0x200 * n, &words, DATA);
    }
    let mut bytes = b.build().bytes;
    let first = get32(&bytes, header(&bytes, ".rd0") + 20);
    for n in 1..15 {
        let h = header(&bytes, &format!(".rd{n}"));
        put32(&mut bytes, h + 20, first);
    }
    bytes.truncate(first as usize + 0x200);
    let exe = Pe::from_bytes(bytes).unwrap();
    let map = MapFile::parse(" 0002:00000000 ??_7V@@6B@ 00402000 \n");
    let err = engine_vtables_from(&exe, &map, &["V"]).unwrap_err();
    assert_eq!(err.message, "vtable too large");
    assert!(err.raw);
}
