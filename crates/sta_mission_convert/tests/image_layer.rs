//! The image layer against PE files and map text the tests author themselves (no game data).

use sta_mission_convert::fixture::{CODE, DATA, Import, PeBuilder};
use sta_mission_convert::map::{
    ExpectedSlot, MapFile, Vtables, check_engine, engine_vtables, engine_vtables_from,
};
use sta_mission_convert::{DecodeError, ErrorKind, FlatImage, Image, Pe};

const BASE: u32 = 0x1000_0000;

fn rdata() -> Vec<u8> {
    let mut d = vec![0u8; 0x20];
    d[..4].copy_from_slice(b"ab\xE9\0");
    d[4..8].copy_from_slice(&1.5f32.to_le_bytes());
    d[8..16].copy_from_slice(&(-2.25f64).to_le_bytes());
    d[16..20].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    d
}

fn sample() -> PeBuilder {
    PeBuilder::new(BASE)
        .section(".text", 0x1000, &[0x90; 0x40], CODE)
        .section(".rdata", 0x2000, &rdata(), DATA)
        .section_sized(".bss", 0x3000, 0x100, &[], DATA)
        .entry(0x1010)
}

#[test]
fn headers_and_sections() {
    let pe = Pe::from_bytes(sample().build().bytes).unwrap();
    assert_eq!(pe.base, BASE);
    assert_eq!(pe.entry_point(), BASE + 0x1010);
    let names: Vec<&str> = pe.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, [".text", ".rdata", ".bss"]);
    assert!(pe.sections[0].is_code());
    assert!(!pe.sections[1].is_code());
    assert_eq!(pe.sections[2].vsize, 0x100);
    assert_eq!(pe.sections[2].raw_size, 0);
    assert_eq!(pe.export_size, 0);
    assert!(pe.imports.is_empty());
    assert!(pe.exports().unwrap().is_empty());
}

#[test]
fn address_mapping() {
    let pe = Pe::from_bytes(sample().build().bytes).unwrap();
    let text = pe.off(0x1000).unwrap();
    assert_eq!(&pe.data[text..text + 2], &[0x90, 0x90]);
    assert_eq!(pe.off_of(BASE + 0x1000), Some(text));
    // Raw data is padded to the file alignment, and the Python `off` accepts the padding.
    assert!(pe.off(0x2000 + 0x1FF).is_some());
    assert_eq!(pe.off(0x2000 + 0x200), None);
    // A section with no raw data maps nowhere but still names its virtual extent.
    assert_eq!(pe.off(0x3000), None);
    assert_eq!(pe.section_of(BASE + 0x3000), Some(".bss"));
    assert_eq!(pe.section_of(BASE + 0x30FF), Some(".bss"));
    assert_eq!(pe.section_of(BASE + 0x3100), None);
    assert_eq!(pe.section_of(BASE + 0x1000), Some(".text"));
    // Below the base: nothing, and no wrap-around into a high section.
    assert_eq!(pe.section_of(BASE - 1), None);
    assert_eq!(pe.off_of(BASE - 1), None);
    assert_eq!(pe.off_of(0), None);
}

#[test]
fn strings_and_numbers() {
    let pe = Pe::from_bytes(sample().build().bytes).unwrap();
    // Latin-1: the byte E9 is the character U+00E9.
    assert_eq!(pe.cstr(BASE + 0x2000).unwrap(), "ab\u{e9}");
    assert_eq!(pe.cstr(BASE + 0x2003).unwrap(), "");
    assert_eq!(pe.f32(BASE + 0x2004).unwrap(), 1.5);
    assert_eq!(pe.f64(BASE + 0x2008).unwrap(), -2.25);
    assert_eq!(pe.dword(BASE + 0x2010), Some(0xDEAD_BEEF));
    assert_eq!(pe.dword(BASE + 0x3000), None);
    let outside = pe.cstr(BASE + 0x3000).unwrap_err();
    assert!(outside.message.contains("outside the file"), "{outside}");
    // A fault the Python tools raise as `DecodeError`, which their callers catch.
    assert_eq!(outside.kind, ErrorKind::Decode);
    let f = pe.f32(BASE + 0x3000).unwrap_err();
    assert!(f.message.contains("outside") && !f.is_raw());
    let d = pe.f64(BASE + 0x3000).unwrap_err();
    assert!(d.message.contains("outside") && !d.is_raw());
}

#[test]
fn unterminated_and_truncated_reads() {
    // One section that fills its raw data with non-zero bytes and ends the file.
    let pe = Pe::from_bytes(
        PeBuilder::new(BASE)
            .section(".d", 0x1000, &[b'x'; 0x200], DATA)
            .build()
            .bytes,
    )
    .unwrap();
    assert_eq!(
        pe.cstr(BASE + 0x1000).unwrap_err(),
        DecodeError::python("ValueError", "subsection not found")
    );
    let last = BASE + 0x1000 + 0x1FE;
    assert_eq!(pe.dword(last), None);
    let o = pe.off_of(last).unwrap();
    assert_eq!(
        pe.f32(last).unwrap_err(),
        sta_mission_convert::image::short_buffer(o, 4, pe.data.len())
    );
    assert_eq!(
        pe.f64(last).unwrap_err(),
        sta_mission_convert::image::short_buffer(o, 8, pe.data.len())
    );
    assert_eq!(pe.dword(last - 2), Some(0x7878_7878));
}

#[test]
fn imports_by_name_and_ordinal() {
    let built = sample()
        .import(
            "one.dll",
            vec![
                Import::name("alpha"),
                Import::Ordinal(7),
                Import::name("gamma"),
            ],
        )
        .import("two.dll", vec![Import::name("delta")])
        .build();
    let pe = Pe::from_bytes(built.bytes).unwrap();
    assert_eq!(built.import_slots.len(), 4);
    let got: Vec<(u32, &str, &str)> = pe
        .imports
        .iter()
        .map(|(&a, (d, s))| (a, d.as_str(), s.as_str()))
        .collect();
    let want = [
        (built.import_slots[0], "one.dll", "alpha"),
        (built.import_slots[1], "one.dll", "#7"),
        (built.import_slots[2], "one.dll", "gamma"),
        (built.import_slots[3], "two.dll", "delta"),
    ];
    assert_eq!(got, want);
    // Slots are consecutive words within one DLL.
    assert_eq!(built.import_slots[1], built.import_slots[0] + 4);
    // The slot's section is the import section; its word is the lookup entry.
    assert_eq!(pe.section_of(built.import_slots[0]), Some(".idata"));
}

#[test]
fn imports_fall_back_to_the_iat_without_a_lookup_table() {
    let built = sample()
        .import("one.dll", vec![Import::name("alpha"), Import::Ordinal(3)])
        .no_lookup_table()
        .build();
    let pe = Pe::from_bytes(built.bytes).unwrap();
    let syms: Vec<&str> = pe.imports.values().map(|(_, s)| s.as_str()).collect();
    assert_eq!(syms, ["alpha", "#3"]);
}

#[test]
fn exports_follow_the_ordinal_table_in_name_order() {
    let pe = Pe::from_bytes(
        sample()
            .export("zeta", 0x1020)
            .export("alpha", 0x1000)
            .export("data_item", 0x2008)
            .build()
            .bytes,
    )
    .unwrap();
    let exports = pe.exports().unwrap();
    let got: Vec<(&str, u32)> = exports.iter().map(|(k, &v)| (k.as_str(), v)).collect();
    assert_eq!(
        got,
        [
            ("zeta", BASE + 0x1020),
            ("alpha", BASE + 0x1000),
            ("data_item", BASE + 0x2008)
        ]
    );
    assert!(pe.export_size > 40);
    assert_eq!(pe.section_of(BASE + 0x2008), Some(".rdata"));
}

#[test]
fn a_repeated_export_name_keeps_its_first_position_and_takes_the_later_address() {
    let pe = Pe::from_bytes(
        sample()
            .export("a", 0x1000)
            .export("b", 0x1004)
            .export("a", 0x1008)
            .build()
            .bytes,
    )
    .unwrap();
    let got: Vec<(String, u32)> = pe.exports().unwrap().into_iter().collect();
    assert_eq!(
        got,
        [
            ("a".to_owned(), BASE + 0x1008),
            ("b".to_owned(), BASE + 0x1004)
        ]
    );
}

#[test]
fn malformed_files() {
    let good = sample().build().bytes;
    assert_eq!(
        Pe::from_bytes(Vec::new()).unwrap_err(),
        DecodeError::new("not a PE file")
    );
    let mut bad = good.clone();
    bad[0] = b'Z';
    assert_eq!(
        Pe::from_bytes(bad).unwrap_err(),
        DecodeError::new("not a PE file")
    );
    let mut bad = good.clone();
    bad[0x40] = b'X';
    assert_eq!(
        Pe::from_bytes(bad).unwrap_err(),
        DecodeError::new("not a PE file")
    );
    let wrong = sample().machine(0x8664).build().bytes;
    assert_eq!(
        Pe::from_bytes(wrong).unwrap_err(),
        DecodeError::new("machine 0x8664, expected i386")
    );
    // The header ends inside the section table.
    assert_eq!(
        Pe::from_bytes(good[..0x150].to_vec()).unwrap_err(),
        DecodeError::raw("truncated PE file")
    );
    // A file that ends inside the last 16 bytes of the last section header still parses (the
    // Python tool reads 24 bytes of each header); the flags it does not read are 0.
    let cut = Pe::from_bytes(good[..0x188 + 40].to_vec()).unwrap();
    assert_eq!(cut.sections.len(), 3);
    assert_eq!(cut.sections[2].characteristics, DATA);
    let cut = Pe::from_bytes(good[..0x188 + 24].to_vec()).unwrap();
    assert_eq!(cut.sections[2].characteristics, 0);
    assert!(Pe::from_bytes(good[..0x188 + 23].to_vec()).is_err());
    // The PE offset points past the file.
    let mut bad = good[..0x80].to_vec();
    bad[0x3C..0x40].copy_from_slice(&0x7000u32.to_le_bytes());
    assert_eq!(
        Pe::from_bytes(bad).unwrap_err(),
        DecodeError::new("not a PE file")
    );
}

#[test]
fn an_import_table_that_runs_off_the_file_is_an_error() {
    let built = sample()
        .import("one.dll", vec![Import::name("alpha")])
        .build();
    // Cut the file inside the import section (it follows the three sections).
    let cut = built.bytes[..0x400 + 0x200 * 2 + 0x40].to_vec();
    assert!(Pe::from_bytes(cut).is_err());
}

#[test]
fn flat_image_works_like_the_python_fake() {
    let mut img = FlatImage::new(BASE, 0x400)
        .with_section(".text", 0x100, 0x100)
        .with_section(".data", 0x200, 0x100);
    img.put(BASE + 0x210, b"hi\0");
    img.put(BASE + 0x220, &0x1234_5678u32.to_le_bytes());
    assert_eq!(img.off(0x210), Some(0x210));
    assert_eq!(img.off(0x400), None);
    assert_eq!(img.section_of(BASE + 0x1FF), Some(".text"));
    assert_eq!(img.section_of(BASE + 0x300), None);
    assert_eq!(img.cstr(BASE + 0x210).unwrap(), "hi");
    assert_eq!(img.dword(BASE + 0x220), Some(0x1234_5678));
    assert_eq!(img.dword(BASE + 0x3FE), None);
}

// ------------------------------------------------------------------ the map and the vtables

#[test]
fn map_lines() {
    let text = [
        "  Address         Publics by Value              Rva+Base       Lib:Object",
        "",
        " 0001:00000000       ??_7Foo@@6B@               00402000 f   x.obj",
        " 0001:00000010       ??_7Foo@@6B@_dup           00402000 f   y.obj",
        "\t0001:00000020\t?bar@Foo@@\t00401020\tf",
        " 0001:0000003       too_short_offset           00401030 f",
        " 0001:00000040       upper_case_address         0040ABCD f",
        " 0001:00000050       short_address              401040 f",
        " 0001:00000060       no_gap                     00401050f",
        " 0001:00000070       latin_\u{e9}                  00401060 f",
        " 0001:00000080       last_line_no_end           00401070",
    ]
    .join("\n");
    let map = MapFile::parse(&text);
    let got: Vec<(u32, &str)> = map.names.iter().map(|(&a, s)| (a, s.as_str())).collect();
    // The first symbol at an address wins; the last line has no line end after the address.
    assert_eq!(
        got,
        [
            (0x402000, "??_7Foo@@6B@"),
            (0x401020, "?bar@Foo@@"),
            (0x401060, "latin_\u{e9}")
        ]
    );
    assert_eq!(map.address_of("?bar@Foo@@"), Some(0x401020));
    assert_eq!(map.address_of("absent"), None);
}

#[test]
fn map_line_ends() {
    let map = MapFile::parse(
        " 0001:00000000 a 00000001 f\r\n 0001:00000000 b 00000002 f\r 0001:00000000 c 00000003 f\n",
    );
    let got: Vec<(u32, &str)> = map.names.iter().map(|(&a, s)| (a, s.as_str())).collect();
    assert_eq!(got, [(1, "a"), (2, "b"), (3, "c")]);
}

fn engine_exe() -> Pe {
    let mut rd = vec![0u8; 0x20];
    // Foo: two named code words, one unnamed, then a word outside the code.
    for (i, w) in [0x40_1010u32, 0x40_1020, 0x40_1030, 0x50_0000]
        .iter()
        .enumerate()
    {
        rd[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    // Bar: two code words, then the word at 0x402018 starts another symbol.
    for (i, w) in [0x40_1010u32, 0x40_1020, 0x40_1010].iter().enumerate() {
        rd[0x10 + i * 4..0x10 + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    Pe::from_bytes(
        PeBuilder::new(0x40_0000)
            .section(".text", 0x1000, &[0x90; 0x100], CODE)
            .section(".rdata", 0x2000, &rd, DATA)
            .build()
            .bytes,
    )
    .unwrap()
}

fn engine_map() -> MapFile {
    MapFile::parse(
        " 0001:00000000 ?a@Foo@@ 00401010 f\n 0001:00000010 ?b@Foo@@ 00401020 f\n \
         0002:00000000 ??_7Foo@@6B@ 00402000 \n 0002:00000010 ??_7Bar@@6B@ 00402010 \n \
         0002:00000018 ?other@@3HA 00402018 \n",
    )
}

#[test]
fn vtable_slots() {
    let vt = engine_vtables_from(&engine_exe(), &engine_map(), &["Bar", "Missing", "Foo"]).unwrap();
    let keys: Vec<&str> = vt.keys().map(String::as_str).collect();
    assert_eq!(keys, ["Bar", "Foo"]);
    assert_eq!(vt["Foo"], ["?a@Foo@@", "?b@Foo@@", "?unnamed_2@@"]);
    assert_eq!(vt["Bar"], ["?a@Foo@@", "?b@Foo@@"]);
}

#[test]
fn vtable_needs_a_text_section() {
    let pe = Pe::from_bytes(
        PeBuilder::new(0x40_0000)
            .section(".rdata", 0x2000, &[0; 8], DATA)
            .build()
            .bytes,
    )
    .unwrap();
    assert_eq!(
        engine_vtables_from(&pe, &MapFile::default(), &[]).unwrap_err(),
        DecodeError::raw("no .text section")
    );
}

#[test]
fn vtable_walking_off_the_file_is_an_error() {
    let map = MapFile::parse(" 0002:00000000 ??_7Foo@@6B@ 00900000 \n");
    assert!(engine_vtables_from(&engine_exe(), &map, &["Foo"]).is_err());
}

#[test]
fn check_engine_messages() {
    let mut vt = Vtables::new();
    vt.insert("Foo".into(), vec!["?a@Foo@@XYZ".into(), "?b@Foo@@".into()]);
    let want = [
        ExpectedSlot {
            class: "Foo",
            slot: 0,
            prefix: "?a@Foo@@",
        },
        ExpectedSlot {
            class: "Foo",
            slot: 1,
            prefix: "?c@Foo@@",
        },
        ExpectedSlot {
            class: "Foo",
            slot: 2,
            prefix: "?d@Foo@@",
        },
        ExpectedSlot {
            class: "Nope",
            slot: 0,
            prefix: "?e@@",
        },
    ];
    assert_eq!(
        check_engine(&vt, &want, true),
        [
            "Foo slot 1: expected ?c@Foo@@, found ?b@Foo@@",
            "Foo slot 2: expected ?d@Foo@@, found none",
            "Nope slot 0: expected ?e@@, found none",
        ]
    );
    assert_eq!(
        check_engine(&vt, &want, false),
        [
            "Foo slot 1: expected ?c@Foo@@",
            "Foo slot 2: expected ?d@Foo@@",
            "Nope slot 0: expected ?e@@"
        ]
    );
    assert!(check_engine(&vt, &want[..1], true).is_empty());
}

#[test]
fn engine_directory_lookup_is_case_insensitive() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("engine_dir_lookup");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = engine_exe();
    std::fs::write(dir.join("ENGINE.EXE"), &exe.data).unwrap();
    // No map yet.
    assert!(
        engine_vtables(&dir, "engine.exe", "engine.map", &["Foo"])
            .unwrap()
            .is_none()
    );
    std::fs::write(
        dir.join("Engine.MAP"),
        " 0001:00000000 ?a@Foo@@ 00401010 f\n 0002:00000000 ??_7Foo@@6B@ 00402000 \n",
    )
    .unwrap();
    let vt = engine_vtables(&dir, "engine.exe", "engine.map", &["Foo"])
        .unwrap()
        .unwrap();
    assert_eq!(vt["Foo"], ["?a@Foo@@", "?unnamed_1@@", "?unnamed_2@@"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_engine_lookup_goes_on_past_io_and_decode_faults_but_not_raw_ones() {
    let base = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("engine_next_root");
    let _ = std::fs::remove_dir_all(&base);
    let map_text = " 0001:00000000 ?a@Foo@@ 00401010 f\n 0002:00000000 ??_7Foo@@6B@ 00402000 \n";
    let dir = |name: &str| {
        let d = base.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    };
    // 1: missing directory (Io). 2: executable that is not a PE (Decode). 3: a PE with no
    // code section (Raw, aborts). 4: a good pair.
    let missing = base.join("missing");
    let not_pe = dir("not_pe");
    std::fs::write(not_pe.join("engine.exe"), b"not a pe").unwrap();
    std::fs::write(not_pe.join("engine.map"), map_text).unwrap();
    let no_text = dir("no_text");
    let rdata_only = PeBuilder::new(0x40_0000)
        .section(".rdata", 0x2000, &[0; 8], DATA)
        .build()
        .bytes;
    std::fs::write(no_text.join("engine.exe"), rdata_only).unwrap();
    std::fs::write(no_text.join("engine.map"), map_text).unwrap();
    let good = dir("good");
    std::fs::write(good.join("engine.exe"), &engine_exe().data).unwrap();
    std::fs::write(good.join("engine.map"), map_text).unwrap();

    let kind = |d: &std::path::Path| {
        engine_vtables(d, "engine.exe", "engine.map", &["Foo"])
            .unwrap_err()
            .kind
    };
    assert_eq!(kind(&missing), ErrorKind::Io);
    assert_eq!(kind(&not_pe), ErrorKind::Decode);
    assert_eq!(kind(&no_text), ErrorKind::Raw);

    // The Python loop: `except (OSError, DecodeError)` tries the next root.
    let lookup = |roots: &[&std::path::Path]| {
        for r in roots {
            match engine_vtables(r, "engine.exe", "engine.map", &["Foo"]) {
                Ok(found) => return Ok(found),
                Err(e) if e.kind != ErrorKind::Raw => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    };
    let found = lookup(&[&missing, &not_pe, &good]).unwrap().unwrap();
    assert_eq!(found["Foo"].len(), 3);
    assert_eq!(
        lookup(&[&missing, &no_text, &good]).unwrap_err().kind,
        ErrorKind::Raw
    );
    let _ = std::fs::remove_dir_all(&base);
}
