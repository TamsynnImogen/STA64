use sta_bzn::{
    BznError,
    a1::{HeaderError, inspect_header},
};

fn header(newline: &str, version: &str, description: &str, binary: &str) -> Vec<u8> {
    format!("version [1] ={newline}{version}{newline}saveGameDesc = {description}{newline}binarySave [1] ={newline}{binary}{newline}").into_bytes()
}

#[test]
fn lossless_header_and_exact_metadata_offset_for_each_known_version() {
    let bytes: Vec<u8> = (128..=255).collect();
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    for newline in ["\n", "\r\n"] {
        for version in 2050..=2053 {
            let mut data = header(newline, &version.to_string(), &hex, "true");
            let offset = data.len();
            // A body may start with newline-like bytes: do not skip them.
            data.extend_from_slice(b"\r\n\0\xff");
            let h = inspect_header(&data).unwrap();
            assert_eq!(h.version, version);
            assert_eq!(h.save_game_desc.as_slice(), bytes);
            assert!(h.binary_save);
            assert_eq!(h.metadata_offset, offset);
            assert_eq!(&data[h.metadata_offset..], b"\r\n\0\xff");
            assert_eq!(
                sta_bzn::parse(&data),
                Err(BznError::UnsupportedArmada1Import { version })
            );
        }
    }
}

#[test]
fn text_body_is_identified_but_not_silently_imported() {
    let data = header("\n", "2050", &"00".repeat(128), "false");
    assert!(!inspect_header(&data).unwrap().binary_save);
    assert_eq!(
        sta_bzn::parse(&data),
        Err(BznError::UnsupportedArmada1Import { version: 2050 })
    );
}

#[test]
fn every_truncated_prefix_is_rejected() {
    for newline in ["\n", "\r\n"] {
        let data = header(newline, "2050", &"00".repeat(128), "true");
        for n in 0..data.len() {
            assert!(inspect_header(&data[..n]).is_err(), "prefix {n} accepted");
            if data[..n].windows(b"saveGameDesc".len()).any(|w| w == b"saveGameDesc") {
                assert!(matches!(sta_bzn::parse(&data[..n]), Err(BznError::Armada1Header(_))));
            }
        }
    }
}

#[test]
fn invalid_description_version_boolean_and_fields_fail_explicitly() {
    let hex = "00".repeat(128);
    for invalid in ["", "00", &"00".repeat(129), &format!("{}gg", "00".repeat(127))] {
        let data = header("\n", "2050", invalid, "true");
        assert!(matches!(
            sta_bzn::parse(&data),
            Err(BznError::Armada1Header(HeaderError::Invalid(_)))
        ));
    }
    for version in ["", "-1", "2050x", "4294967296"] {
        assert!(matches!(
            inspect_header(&header("\n", version, &hex, "true")),
            Err(HeaderError::Invalid(_))
        ));
    }
    for version in [0, 2049, 2054, u32::MAX] {
        assert_eq!(
            inspect_header(&header("\n", &version.to_string(), &hex, "true")),
            Err(HeaderError::UnsupportedVersion(version))
        );
    }
    for binary in ["", "1", "TRUE", "true "] {
        assert!(matches!(
            sta_bzn::parse(&header("\n", "2050", &hex, binary)),
            Err(BznError::Armada1Header(HeaderError::Invalid(_)))
        ));
    }
    let valid = String::from_utf8(header("\n", "2050", &hex, "true")).unwrap();
    for (from, to) in [
        ("version [1] =", "version [2] ="),
        ("saveGameDesc = ", "saveGameDesc: "),
        ("binarySave [1] =", "binarySave [2] ="),
    ] {
        let invalid = valid.replace(from, to);
        assert!(matches!(
            sta_bzn::parse(invalid.as_bytes()),
            Err(BznError::Armada1Header(HeaderError::Invalid(_)))
        ));
    }
    let missing_description = b"version [1] =\n2050\nbinarySave [1] =\ntrue\n";
    assert!(matches!(
        sta_bzn::parse(missing_description),
        Err(BznError::Armada1Header(HeaderError::Invalid(_)))
    ));
}

#[test]
fn marker_in_a2_text_or_binary_body_does_not_change_dispatch() {
    let text = b"version [1] =\n2100\nBinaryMode [1] =\nfalse\nMissionName = saveGameDesc\n";
    let map = sta_bzn::parse(text).unwrap();
    assert_eq!(map.version, 2100);
    assert!(!map.binary);
    let mut binary = b"version [1] =\n2100\nBinaryMode [1] =\ntrue\n".to_vec();
    let value = b"saveGameDesc\0";
    binary.extend_from_slice(&2_u32.to_le_bytes());
    binary.extend_from_slice(&(value.len() as u32).to_le_bytes());
    binary.extend_from_slice(value);
    let map = sta_bzn::parse(&binary).unwrap();
    assert_eq!(map.version, 2100);
    assert!(map.binary);
}

#[test]
fn a2_fields_with_similar_names_keep_the_existing_dispatch() {
    for field in ["saveGameDescription", "binarySaveMode"] {
        for binary in [false, true] {
            let control = format!("version [1] =\n2100\nBinaryMode [1] =\n{binary}\n");
            let collision = format!("version [1] =\n2100\n{field} = ignored\nBinaryMode [1] =\n{binary}\n");
            assert_eq!(sta_bzn::parse(collision.as_bytes()), sta_bzn::parse(control.as_bytes()));
        }
    }
}

#[test]
#[ignore = "requires STA64_TEST_A1_BZN_DIR pointing at user-owned original A1 maps"]
fn installed_a1_maps_have_explicit_unsupported_results() {
    let root = std::env::var_os("STA64_TEST_A1_BZN_DIR").expect("set STA64_TEST_A1_BZN_DIR");
    let mut count = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("bzn")) {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let h = inspect_header(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(h.binary_save, "{}", path.display());
        assert!(h.metadata_offset < bytes.len());
        assert_eq!(
            sta_bzn::parse(&bytes),
            Err(BznError::UnsupportedArmada1Import { version: h.version }),
            "{}",
            path.display()
        );
        count += 1;
    }
    assert!(count > 0, "no A1 maps found");
    eprintln!("A1 headers inspected: {count}; explicitly unsupported imports: {count}; no map-body support claimed");
}
