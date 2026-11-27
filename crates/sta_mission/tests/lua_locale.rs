//! A process locale set by some host library cannot change what a script sees (#1300): number
//! text and parsing, `string.lower/upper` and the pattern classes stay those of the C locale.
//! Its own test binary, because the locale is process-wide. Skips (with a note) when the machine
//! has none of the locales tried.

use std::ffi::{CStr, CString, c_char, c_int};

use sta_mission::sandbox::{new_lua, refill};

unsafe extern "C" {
    fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
}

#[cfg(target_os = "linux")]
const LC_ALL: c_int = 6;
#[cfg(not(target_os = "linux"))]
const LC_ALL: c_int = 0;

fn try_locale(names: &[&str]) -> Option<String> {
    for name in names {
        let c = CString::new(*name).unwrap();
        // SAFETY: setlocale with a valid C string; this binary runs one test.
        let got = unsafe { setlocale(LC_ALL, c.as_ptr()) };
        if !got.is_null() {
            return Some(unsafe { CStr::from_ptr(got) }.to_string_lossy().into_owned());
        }
    }
    None
}

#[test]
fn a_host_locale_does_not_reach_scripts() {
    let check = || {
        let lua = new_lua();
        refill(&lua);
        lua.load(
            "return tostring(1.5) .. '|' .. tostring(2.0) .. '|' .. tostring(tonumber('1.25')) .. '|' .. tostring(tonumber('1,25')) \
             .. '|' .. string.format('%.2f %g %a', 1.5, 0.25, 0.5) .. '|' .. ('\\233abc'):upper():byte(1) .. ('ABC\\201'):lower() \
             .. '|' .. tostring(('\\233'):find('%a')) .. tostring(('\\233'):find('%W')) .. '|' .. (1.5 .. '')",
        )
        .eval::<mlua::LuaString>()
        .unwrap()
        .as_bytes()
        .to_vec()
    };
    let want = check();
    assert_eq!(&want[..], b"1.5|2.0|1.25|nil|1.50 0.25 0x1p-1|233abc\xc9|nil1|1.5");
    // Comma decimal point and Latin-1 classes, under the names the platforms use.
    let names = [
        "de_DE.ISO-8859-1", "de_DE.ISO8859-1", "de_DE.iso88591", "de_DE.UTF-8", "de_DE.utf8", "de_DE", "fr_FR.UTF-8", "fr_FR",
        "German_Germany.1252", "de-DE",
    ];
    let Some(set) = try_locale(&names) else {
        eprintln!("no comma-decimal locale on this machine; only the C locale was checked");
        return;
    };
    let got = check();
    let c = CString::new("C").unwrap();
    // SAFETY: as above.
    unsafe { setlocale(LC_ALL, c.as_ptr()) };
    assert_eq!(got, want, "under locale {set}");
}
