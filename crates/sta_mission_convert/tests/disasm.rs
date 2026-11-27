//! Install-gated: the listing of every mission DLL of an install equals what GNU objdump 2.47
//! prints (`objdump -d -M intel --no-show-raw-insn`, the lines the Python tools keep). Only
//! counts and the address of the first difference are reported, never text. The installs are
//! named by `STA64_TEST_A2_STOCK_INSTALL` (Armada II) and `STA64_TEST_A1_INSTALL` (Armada I);
//! a test without its variable, without objdump or with another objdump version skips.
//! `STA64_DISASM_DUMP=<dir>` writes our listing of each file there (a scratch directory, never an
//! install) for diffing by hand.

use std::path::{Path, PathBuf};
use std::process::Command;

use sta_mission_convert::Pe;
use sta_mission_convert::disasm::{Listing, parse_objdump};

/// Every file under `dir`, at any depth (like the Python tools' `os.walk`; links are not followed).
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&p, out),
            _ => out.push(p),
        }
    }
}

fn objdump_ok() -> bool {
    match Command::new("objdump").arg("--version").output() {
        Ok(o) => {
            let v = String::from_utf8_lossy(&o.stdout);
            let first = v.lines().next().unwrap_or("");
            if first.contains(" 2.47") {
                true
            } else {
                eprintln!("objdump is not version 2.47: skipped");
                false
            }
        }
        Err(_) => {
            eprintln!("objdump not found: skipped");
            false
        }
    }
}

fn run(var: &str) {
    let Ok(root) = std::env::var(var) else {
        eprintln!("{var} not set: skipped");
        return;
    };
    if !objdump_ok() {
        return;
    }
    let dump = std::env::var("STA64_DISASM_DUMP").ok().map(PathBuf::from);
    let mut files = Vec::new();
    walk(Path::new(&root), &mut files);
    files.retain(|p| {
        p.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dsl") || e.eq_ignore_ascii_case("drl"))
    });
    files.sort();
    assert!(!files.is_empty(), "no mission DLL under the install");
    let (mut lines, mut differing, mut bad_files) = (0usize, 0usize, 0usize);
    let t0 = std::time::Instant::now();
    let mut listing_time = std::time::Duration::ZERO;
    for (n, p) in files.iter().enumerate() {
        let out = Command::new("objdump")
            .args(["-d", "-M", "intel", "--no-show-raw-insn"])
            .arg(p)
            .output()
            .expect("objdump runs");
        let want = parse_objdump(&String::from_utf8_lossy(&out.stdout));
        let pe = Pe::open(p).unwrap_or_else(|e| panic!("file {n} does not parse: {e}"));
        let t1 = std::time::Instant::now();
        let got = Listing::from_image(&pe).expect("no symbol table").ins;
        listing_time += t1.elapsed();
        if let Some(d) = &dump {
            let text: String = got.iter().map(|(a, t)| format!("{a:x}:\t{t}\n")).collect();
            std::fs::write(d.join(format!("{var}-{n:03}.txt")), text).expect("dump written");
        }
        lines += want.len();
        let mut diff = 0usize;
        let mut first = None;
        let (mut i, mut j) = (0, 0);
        // Lines are compared in order; a missing or extra line counts once.
        while i < want.len() || j < got.len() {
            match (want.get(i), got.get(j)) {
                (Some(a), Some(b)) if a == b => {
                    i += 1;
                    j += 1;
                }
                (Some(a), Some(b)) => {
                    diff += 1;
                    first.get_or_insert(a.0.min(b.0));
                    if a.0 == b.0 {
                        i += 1;
                        j += 1;
                    } else if a.0 < b.0 {
                        i += 1;
                    } else {
                        j += 1;
                    }
                }
                (Some(a), None) => {
                    diff += 1;
                    first.get_or_insert(a.0);
                    i += 1;
                }
                (None, Some(b)) => {
                    diff += 1;
                    first.get_or_insert(b.0);
                    j += 1;
                }
                (None, None) => unreachable!(),
            }
        }
        if diff > 0 {
            bad_files += 1;
            eprintln!(
                "file {n}: {diff} differing lines, first at {:#x}",
                first.unwrap_or(0)
            );
        }
        differing += diff;
    }
    eprintln!(
        "{var}: {} files, {lines} lines compared, {differing} differing, {bad_files} files differ, \
         listing time {listing_time:?} of {:?}",
        files.len(),
        t0.elapsed()
    );
    assert_eq!(differing, 0, "the listing differs from objdump's");
}

/// Dev aid: `STA64_DISASM_RAW=<file>` is a flat blob of authored bytes (never game data); its
/// listing at 0x10000000 is written to `<STA64_DISASM_DUMP>/raw.txt`.
#[test]
fn raw_blob_listing_dump() {
    let (Ok(f), Ok(d)) = (
        std::env::var("STA64_DISASM_RAW"),
        std::env::var("STA64_DISASM_DUMP"),
    ) else {
        return;
    };
    let data = std::fs::read(f).expect("blob readable");
    let mut got = Vec::new();
    sta_mission_convert::disasm::disassemble_bytes(0x1000_0000, &data, &mut got);
    let text: String = got.iter().map(|(a, t)| format!("{a:x}:\t{t}\n")).collect();
    std::fs::write(PathBuf::from(d).join("raw.txt"), text).expect("dump written");
}

#[test]
fn a2_install_listing_equals_objdump() {
    run("STA64_TEST_A2_STOCK_INSTALL");
}

#[test]
fn a1_install_listing_equals_objdump() {
    run("STA64_TEST_A1_INSTALL");
}

/// Hand-assembled byte sequences (written from the public instruction set documentation) and
/// the text the reference tool printed for the same bytes at 0x10001000.
#[allow(clippy::type_complexity)]
const HAND: &[(&str, &[&str])] = &[
    ("55", &["push   ebp"]),
    ("8b ec", &["mov    ebp,esp"]),
    ("83 c4 fc", &["add    esp,0xfffffffc"]),
    ("6a ff", &["push   0xffffffff"]),
    ("c2 0c 00", &["ret    0xc"]),
    ("8b 54 24 04", &["mov    edx,DWORD PTR [esp+0x4]"]),
    ("3a 01", &["cmp    al,BYTE PTR [ecx]"]),
    ("3d 00 10 00 00", &["cmp    eax,0x1000"]),
    ("e8 fb 0f 00 00", &["call   0x10002000"]),
    ("eb 05", &["jmp    0x10001007"]),
    ("0f 84 00 01 00 00", &["je     0x10001106"]),
    ("ff 15 00 30 00 10", &["call   DWORD PTR ds:0x10003000"]),
    ("a1 04 30 00 10", &["mov    eax,ds:0x10003004"]),
    ("a3 04 30 00 10", &["mov    ds:0x10003004,eax"]),
    ("8b 15 04 30 00 10", &["mov    edx,DWORD PTR ds:0x10003004"]),
    ("64 a1 00 00 00 00", &["mov    eax,fs:0x0"]),
    ("64 89 25 00 00 00 00", &["mov    DWORD PTR fs:0x0,esp"]),
    ("f3 ab", &["rep stos DWORD PTR es:[edi],eax"]),
    ("f3 a5", &["rep movs DWORD PTR es:[edi],DWORD PTR ds:[esi]"]),
    ("a4", &["movs   BYTE PTR es:[edi],BYTE PTR ds:[esi]"]),
    ("f3 a6", &["repz cmps BYTE PTR ds:[esi],BYTE PTR es:[edi]"]),
    ("f2 ae", &["repnz scas al,BYTE PTR es:[edi]"]),
    ("ac", &["lods   al,BYTE PTR ds:[esi]"]),
    ("d1 f8", &["sar    eax,1"]),
    ("d3 e0", &["shl    eax,cl"]),
    ("c1 e0 02", &["shl    eax,0x2"]),
    ("0f b6 c0", &["movzx  eax,al"]),
    ("0f be 00", &["movsx  eax,BYTE PTR [eax]"]),
    ("0f bf c0", &["movsx  eax,ax"]),
    ("66 8b 45 fc", &["mov    ax,WORD PTR [ebp-0x4]"]),
    ("66 c7 45 fc 01 00", &["mov    WORD PTR [ebp-0x4],0x1"]),
    ("8d 74 26 00", &["lea    esi,[esi+eiz*1+0x0]"]),
    ("0f 1f 40 00", &["nop    DWORD PTR [eax+0x0]"]),
    ("90", &["nop"]),
    ("66 90", &["xchg   ax,ax"]),
    ("d9 45 fc", &["fld    DWORD PTR [ebp-0x4]"]),
    ("dd d8", &["fstp   st(0)"]),
    ("d8 c1", &["fadd   st,st(1)"]),
    ("dc c1", &["fadd   st(1),st"]),
    ("de c1", &["faddp  st(1),st"]),
    ("d9 c9", &["fxch   st(1)"]),
    ("df e0", &["fnstsw ax"]),
    ("9b d9 7d fc", &["fstcw  WORD PTR [ebp-0x4]"]),
    ("9b dd 7d f8", &["fstsw  WORD PTR [ebp-0x8]"]),
    ("9b 90", &["fwait", "nop"]),
    ("9b d9 c0", &["fld    st(0)"]),
    ("cc", &["int3"]),
    ("0f 0b", &["ud2"]),
    ("8d 04 85 00 10 00 00", &["lea    eax,[eax*4+0x1000]"]),
    (
        "ff 24 85 00 20 00 10",
        &["jmp    DWORD PTR [eax*4+0x10002000]"],
    ),
    ("2e 8b 00", &["mov    eax,DWORD PTR cs:[eax]"]),
    ("36 90", &["ss nop"]),
    ("f0 ff 00", &["lock inc DWORD PTR [eax]"]),
    ("d7", &["xlat   BYTE PTR ds:[ebx]"]),
    ("8b 45 f0", &["mov    eax,DWORD PTR [ebp-0x10]"]),
    ("8b 45 10", &["mov    eax,DWORD PTR [ebp+0x10]"]),
    ("89 04 24", &["mov    DWORD PTR [esp],eax"]),
    ("c6 45 ff 00", &["mov    BYTE PTR [ebp-0x1],0x0"]),
    ("c7 00 ff ff ff ff", &["mov    DWORD PTR [eax],0xffffffff"]),
    (
        "8b 04 8d f0 ff ff ff",
        &["mov    eax,DWORD PTR [ecx*4-0x10]"],
    ),
    ("8d 44 24 fc", &["lea    eax,[esp-0x4]"]),
    ("60", &["pusha"]),
    ("61", &["popa"]),
    ("9c", &["pushf"]),
    ("9d", &["popf"]),
    ("99", &["cdq"]),
    ("98", &["cwde"]),
    ("66 98", &["cbw"]),
    ("cf", &["iret"]),
    ("d9 05 00 10 00 10", &["fld    DWORD PTR ds:0x10001000"]),
    ("dd 05 00 10 00 10", &["fld    QWORD PTR ds:0x10001000"]),
    ("db 2d 00 10 00 10", &["fld    TBYTE PTR ds:0x10001000"]),
    ("df 2d 00 10 00 10", &["fild   QWORD PTR ds:0x10001000"]),
    ("da e9", &["fucompp"]),
    ("db e9", &["fucomi st,st(1)"]),
    ("df e9", &["fucomip st,st(1)"]),
    ("d9 e8", &["fld1"]),
    ("d9 ee", &["fldz"]),
    ("de d9", &["fcompp"]),
    ("dd 45 f8", &["fld    QWORD PTR [ebp-0x8]"]),
    ("d8 4d fc", &["fmul   DWORD PTR [ebp-0x4]"]),
    ("dc 4d fc", &["fmul   QWORD PTR [ebp-0x4]"]),
    ("f6 c4 41", &["test   ah,0x41"]),
    ("a8 01", &["test   al,0x1"]),
    ("69 c0 e8 03 00 00", &["imul   eax,eax,0x3e8"]),
    ("6b c0 0c", &["imul   eax,eax,0xc"]),
    ("0f af c1", &["imul   eax,ecx"]),
    ("f7 f9", &["idiv   ecx"]),
    ("c9", &["leave"]),
    ("c3", &["ret"]),
    ("cb", &["retf"]),
    ("0f a2", &["cpuid"]),
    ("0f 31", &["rdtsc"]),
    ("e4 60", &["in     al,0x60"]),
    ("ec", &["in     al,dx"]),
    ("ee", &["out    dx,al"]),
    ("0f 20 c0", &["mov    eax,cr0"]),
    ("8c d8", &["mov    eax,ds"]),
    ("8e d8", &["mov    ds,eax"]),
    ("1e", &["push   ds"]),
    ("07", &["pop    es"]),
    ("ea 00 10 00 10 23 00", &["jmp    0x23:0x10001000"]),
    ("ff 28", &["jmp    FWORD PTR [eax]"]),
    ("ff 18", &["call   FWORD PTR [eax]"]),
    ("c4 00", &["les    eax,FWORD PTR [eax]"]),
    ("0f 01 10", &["lgdtd  [eax]"]),
    ("0f 18 00", &["prefetchnta BYTE PTR [eax]"]),
    ("0f ae f8", &["sfence"]),
    ("0f c7 08", &["cmpxchg8b QWORD PTR [eax]"]),
    ("0f c8", &["bswap  eax"]),
    ("87 c1", &["xchg   ecx,eax"]),
    ("91", &["xchg   ecx,eax"]),
    ("86 c4", &["xchg   ah,al"]),
    ("0f a3 c8", &["bt     eax,ecx"]),
    ("0f ab c8", &["bts    eax,ecx"]),
    ("0f b3 c8", &["btr    eax,ecx"]),
    ("f3 90", &["pause"]),
    ("f3 0f 10 00", &["movss  xmm0,DWORD PTR [eax]"]),
    ("66 0f 6f 00", &["movdqa xmm0,XMMWORD PTR [eax]"]),
    ("0f 6f 00", &["movq   mm0,QWORD PTR [eax]"]),
    ("0f 28 00", &["movaps xmm0,XMMWORD PTR [eax]"]),
];

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).expect("hex byte"))
        .collect()
}

#[test]
fn hand_assembled_bytes_print_as_the_reference_tool_printed_them() {
    let mut failures = Vec::new();
    for (bytes, want) in HAND {
        let mut got = Vec::new();
        sta_mission_convert::disasm::disassemble_bytes(0x1000_1000, &hex(bytes), &mut got);
        let got: Vec<&str> = got.iter().map(|(_, t)| t.as_str()).collect();
        if got != *want {
            failures.push(format!("{bytes}: want {want:?}, got {got:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// More hand-assembled sequences with the instruction offsets the reference tool listed for them
/// (zero runs, instructions cut off by the end of the data, prefix words, `(bad)`, `fwait`,
/// addressing, branches, immediates, x87 and string forms). Offsets are from the first byte.
#[allow(clippy::type_complexity)]
const RULES: &[(&str, &[(u32, &str)])] = &[
    ("55 00 90", &[(0, "push   ebp"), (1, ".byte 0"), (2, "nop")]),
    (
        "55 00 00 90",
        &[
            (0, "push   ebp"),
            (1, "add    BYTE PTR [eax],al"),
            (3, "nop"),
        ],
    ),
    (
        "55 00 00 00 90",
        &[
            (0, "push   ebp"),
            (1, "add    BYTE PTR [eax],al"),
            (3, ".byte 0"),
            (4, "nop"),
        ],
    ),
    (
        "55 00 00 00 00 90",
        &[
            (0, "push   ebp"),
            (1, "add    BYTE PTR [eax],al"),
            (3, "add    BYTE PTR [eax],al"),
            (5, "nop"),
        ],
    ),
    (
        "55 00 00 00 00 00 00 90",
        &[
            (0, "push   ebp"),
            (1, "add    BYTE PTR [eax],al"),
            (3, "add    BYTE PTR [eax],al"),
            (5, "add    BYTE PTR [eax],al"),
            (7, "nop"),
        ],
    ),
    (
        "55 00 00 00 00 00 00 00 90",
        &[
            (0, "push   ebp"),
            (1, "add    BYTE PTR [eax],al"),
            (3, "add    BYTE PTR [eax],al"),
            (5, "add    BYTE PTR [eax],al"),
            (7, ".byte 0"),
            (8, "nop"),
        ],
    ),
    (
        "55 00 00 00 00 00 00 00 00 90",
        &[(0, "push   ebp"), (9, "nop")],
    ),
    (
        "55 00 00 00 00 00 00 00 00 00 90",
        &[(0, "push   ebp"), (9, ".byte 0"), (10, "nop")],
    ),
    (
        "55 00 00 00 00 00 00 00 00 00 00 90",
        &[
            (0, "push   ebp"),
            (9, "add    BYTE PTR [eax],al"),
            (11, "nop"),
        ],
    ),
    (
        "55 00 00 00 00 00 00 00 00 00 00 00 90",
        &[
            (0, "push   ebp"),
            (9, "add    BYTE PTR [eax],al"),
            (11, ".byte 0"),
            (12, "nop"),
        ],
    ),
    (
        "55 00 00 00 00 00 00 00 00 00 00 00 00 90",
        &[(0, "push   ebp"), (13, "nop")],
    ),
    (
        "55 00 00 00 00 00 00 00 00 00 00 00 00 00 90",
        &[(0, "push   ebp"), (13, ".byte 0"), (14, "nop")],
    ),
    (
        "55 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 90",
        &[(0, "push   ebp"), (17, "nop")],
    ),
    ("55 00", &[(0, "push   ebp")]),
    ("55 00 00", &[(0, "push   ebp")]),
    (
        "55 00 00 00",
        &[(0, "push   ebp"), (1, "add    BYTE PTR [eax],al")],
    ),
    (
        "55 00 00 00 00",
        &[(0, "push   ebp"), (1, "add    BYTE PTR [eax],al")],
    ),
    (
        "55 00 00 00 00 00 00 00",
        &[
            (0, "push   ebp"),
            (1, "add    BYTE PTR [eax],al"),
            (3, "add    BYTE PTR [eax],al"),
            (5, "add    BYTE PTR [eax],al"),
        ],
    ),
    ("55 00 00 00 00 00 00 00 00", &[(0, "push   ebp")]),
    ("55 00 00 00 00 00 00 00 00 00", &[(0, "push   ebp")]),
    ("00 00 00 00 00 00 00 00 90", &[(8, "nop")]),
    (
        "00 00 00 00 00 00 00 00 00 55",
        &[(8, ".byte 0"), (9, "push   ebp")],
    ),
    ("0f 0f", &[(0, ".byte 0xf"), (1, ".byte 0xf")]),
    ("62 c0", &[(0, ".byte 0x62"), (1, ".byte 0xc0")]),
    ("8d c0", &[(0, "lea    eax,(bad)"), (1, ".byte 0xc0")]),
    ("66", &[(0, "data16")]),
    ("f0", &[(0, "lock")]),
    ("0f", &[(0, ".byte 0xf")]),
    ("e8 01", &[(0, ".byte 0xe8"), (1, ".byte 0x1")]),
    (
        "ff 15 74",
        &[(0, ".byte 0xff"), (1, ".byte 0x15"), (2, ".byte 0x74")],
    ),
    ("9b", &[(0, "fwait")]),
    ("67", &[(0, "addr16")]),
    ("0f 38", &[(0, ".byte 0xf"), (1, ".byte 0x38")]),
    (
        "c7 f8 01",
        &[(0, ".byte 0xc7"), (1, "clc"), (2, ".byte 0x1")],
    ),
    ("66 66 90", &[(0, "data16 xchg ax,ax")]),
    ("f3 66 90", &[(0, "data16 pause")]),
    ("f0 f0 40", &[(0, "lock lock inc eax")]),
    ("2e 36 90", &[(0, "cs ss nop")]),
    ("67 90", &[(0, "addr16 nop")]),
    ("67 a4", &[(0, "movs   BYTE PTR es:[di],BYTE PTR ds:[si]")]),
    ("f2 e8 00 00 00 00", &[(0, "bnd call 0x10001006")]),
    ("f3 c3", &[(0, "repz ret")]),
    (
        "f3 a4",
        &[(0, "rep movs BYTE PTR es:[edi],BYTE PTR ds:[esi]")],
    ),
    (
        "f3 a6",
        &[(0, "repz cmps BYTE PTR ds:[esi],BYTE PTR es:[edi]")],
    ),
    (
        "f2 a4",
        &[(0, "repnz movs BYTE PTR es:[edi],BYTE PTR ds:[esi]")],
    ),
    (
        "f2 a6",
        &[(0, "repnz cmps BYTE PTR ds:[esi],BYTE PTR es:[edi]")],
    ),
    ("f3 ac", &[(0, "rep lods al,BYTE PTR ds:[esi]")]),
    ("f3 aa", &[(0, "rep stos BYTE PTR es:[edi],al")]),
    ("f3 6c", &[(0, "rep ins BYTE PTR es:[edi],dx")]),
    ("f3 6e", &[(0, "rep outs dx,BYTE PTR ds:[esi]")]),
    ("66 74 05", &[(0, "data16 je 0x10001008")]),
    ("66 eb 05", &[(0, "data16 jmp 0x10001008")]),
    ("66 c3", &[(0, "retw")]),
    ("66 e8 00 10", &[(0, "callw  0x2004")]),
    ("66 60", &[(0, "pushaw")]),
    ("66 61", &[(0, "popaw")]),
    ("66 9c", &[(0, "pushfw")]),
    ("66 9d", &[(0, "popfw")]),
    ("66 cf", &[(0, "iretw")]),
    ("66 c9", &[(0, "leavew")]),
    ("66 06", &[(0, "pushw  es")]),
    ("66 0f 84 00 10", &[(0, "jew    0x2005")]),
    (
        "f0 f3 01 10",
        &[(0, "lock xrelease add DWORD PTR [eax],edx")],
    ),
    (
        "f0 f2 01 10",
        &[(0, "lock xacquire add DWORD PTR [eax],edx")],
    ),
    ("f3 88 00", &[(0, "xrelease mov BYTE PTR [eax],al")]),
    ("f3 89 00", &[(0, "xrelease mov DWORD PTR [eax],eax")]),
    ("f2 87 00", &[(0, "xacquire xchg DWORD PTR [eax],eax")]),
    ("36 8b 00", &[(0, "mov    eax,DWORD PTR ss:[eax]")]),
    ("2e 8b 00", &[(0, "mov    eax,DWORD PTR cs:[eax]")]),
    ("3e 8b 00", &[(0, "mov    eax,DWORD PTR ds:[eax]")]),
    ("26 8b 00", &[(0, "mov    eax,DWORD PTR es:[eax]")]),
    ("64 8b 00", &[(0, "mov    eax,DWORD PTR fs:[eax]")]),
    ("65 8b 00", &[(0, "mov    eax,DWORD PTR gs:[eax]")]),
    ("64 90", &[(0, "fs nop")]),
    ("65 a1 00 00 00 00", &[(0, "mov    eax,gs:0x0")]),
    ("2e a1 04 30 00 10", &[(0, "mov    eax,cs:0x10003004")]),
    ("f0 02 00", &[(0, "lock add al,BYTE PTR [eax]")]),
    ("f0 40", &[(0, "lock inc eax")]),
    ("f0 c3", &[(0, "lock ret")]),
    ("f0 d9 c0", &[(0, "lock fld st(0)")]),
    ("f0 d8 00", &[(0, "lock fadd DWORD PTR [eax]")]),
    (
        "66 a5",
        &[(0, "movs   WORD PTR es:[edi],WORD PTR ds:[esi]")],
    ),
    ("66 ab", &[(0, "stos   WORD PTR es:[edi],ax")]),
    (
        "64 a4",
        &[(0, "movs   BYTE PTR es:[edi],BYTE PTR fs:[esi]")],
    ),
    (
        "26 a4",
        &[(0, "movs   BYTE PTR es:[edi],BYTE PTR es:[esi]")],
    ),
    ("2e ac", &[(0, "lods   al,BYTE PTR cs:[esi]")]),
    (
        "f3 66 a5",
        &[(0, "rep movs WORD PTR es:[edi],WORD PTR ds:[esi]")],
    ),
    ("66 98", &[(0, "cbw")]),
    ("66 99", &[(0, "cwd")]),
    (
        "66 8b 05 00 10 00 00",
        &[(0, "mov    ax,WORD PTR ds:0x1000")],
    ),
    ("66 b8 34 12", &[(0, "mov    ax,0x1234")]),
    ("66 05 34 12", &[(0, "add    ax,0x1234")]),
    ("66 83 c0 ff", &[(0, "add    ax,0xffff")]),
    ("66 6a ff", &[(0, "pushw  0xffff")]),
    ("66 68 34 12", &[(0, "pushw  0x1234")]),
    ("66 ff 00", &[(0, "inc    WORD PTR [eax]")]),
    ("66 c7 00 34 12", &[(0, "mov    WORD PTR [eax],0x1234")]),
    ("f2 ff 20", &[(0, "bnd jmp DWORD PTR [eax]")]),
    ("f2 c2 04 00", &[(0, "bnd ret 0x4")]),
    ("f2 0f 84 00 10 00 00", &[(0, "bnd je 0x10002007")]),
    ("f2 74 05", &[(0, "bnd je 0x10001008")]),
    ("66 e0 05", &[(0, "data16 loopne 0x10001008")]),
    ("67 e3 05", &[(0, "jcxz   0x10001008")]),
    ("67 e2 05", &[(0, "addr16 loop 0x10001008")]),
    ("67 a0 00 10", &[(0, "addr16 mov al,ds:0x1000")]),
    ("67 a1 00 10", &[(0, "addr16 mov eax,ds:0x1000")]),
    ("67 a3 00 10", &[(0, "addr16 mov ds:0x1000,eax")]),
    ("d9 d1 90", &[(0, "(bad)"), (2, "nop")]),
    ("dc d1 90", &[(0, "(bad)"), (2, "nop")]),
    ("d9 08 90", &[(0, "(bad) [eax]"), (2, "nop")]),
    (
        "d9 8a 00 10 00 00 90",
        &[(0, "(bad) [edx+0x1000]"), (6, "nop")],
    ),
    ("db 36 90", &[(0, "(bad) [esi]"), (2, "nop")]),
    ("db 67 01 90", &[(0, "(bad) [edi+0x1]"), (3, "nop")]),
    (
        "dd 2d 00 10 00 00 90",
        &[(0, "(bad) ds:0x1000"), (6, "nop")],
    ),
    (
        "8d c0 90 90",
        &[
            (0, "lea    eax,(bad)"),
            (1, ".byte 0xc0"),
            (2, "nop"),
            (3, "nop"),
        ],
    ),
    (
        "8d dd 90 90",
        &[
            (0, "lea    ebx,(bad)"),
            (1, ".byte 0xdd"),
            (2, "nop"),
            (3, "nop"),
        ],
    ),
    (
        "ff e8 90 90",
        &[
            (0, "jmp    (bad)"),
            (1, ".byte 0xe8"),
            (2, "nop"),
            (3, "nop"),
        ],
    ),
    (
        "ff d8 90 90",
        &[
            (0, "call   (bad)"),
            (1, ".byte 0xd8"),
            (2, "nop"),
            (3, "nop"),
        ],
    ),
    (
        "8f 08 90 90 90 90 90",
        &[(0, "(bad)"), (4, "nop"), (5, "nop"), (6, "nop")],
    ),
    (
        "8f 1d 8c 90",
        &[
            (0, "(bad)"),
            (1, ".byte 0x1d"),
            (2, ".byte 0x8c"),
            (3, "nop"),
        ],
    ),
    (
        "c5 c0 90 90 90 90",
        &[
            (0, ".byte 0xc5"),
            (1, ".byte 0xc0"),
            (2, "nop"),
            (3, "nop"),
            (4, "nop"),
            (5, "nop"),
        ],
    ),
    (
        "c4 c2 12 20 aa c0 90 90 90",
        &[
            (0, "(bad)"),
            (4, "stos   BYTE PTR es:[edi],al"),
            (5, ".byte 0xc0"),
            (6, "nop"),
            (7, "nop"),
            (8, "nop"),
        ],
    ),
    (
        "c4 e1 34 21 55 e0 90 90 90",
        &[
            (0, "(bad)"),
            (4, "push   ebp"),
            (5, "loopne 0x10000f97"),
            (7, "nop"),
            (8, "nop"),
        ],
    ),
    (
        "c4 00 90 90",
        &[(0, "les    eax,FWORD PTR [eax]"), (2, "nop"), (3, "nop")],
    ),
    (
        "c5 00 90 90",
        &[(0, "lds    eax,FWORD PTR [eax]"), (2, "nop"), (3, "nop")],
    ),
    ("0f 04 90", &[(0, "(bad)"), (2, "nop")]),
    ("66 0f 04 90", &[(0, "data16 (bad)"), (3, "nop")]),
    ("f0 0f 04 90", &[(0, "lock (bad)"), (3, "nop")]),
    ("ff f8 90", &[(0, "(bad)"), (1, "clc"), (2, "nop")]),
    ("ff 38 90", &[(0, "(bad)"), (1, ".byte 0x38"), (2, "nop")]),
    (
        "c6 38 90 90",
        &[(0, "xabort (bad),0x38"), (2, "nop"), (3, "nop")],
    ),
    (
        "c6 78 90 90",
        &[(0, "xabort (bad),0x78"), (2, "nop"), (3, "nop")],
    ),
    (
        "c6 be 90 90",
        &[(0, "(bad)"), (1, ".byte 0xbe"), (2, "nop"), (3, "nop")],
    ),
    (
        "c7 38 02 20 3e 40 90",
        &[
            (0, "xbegin (bad),0x4e20123d"),
            (5, "inc    eax"),
            (6, "nop"),
        ],
    ),
    (
        "c7 be 02 20 3e c0 90",
        &[(0, "(bad)"), (1, "mov    esi,0xc03e2002"), (6, "nop")],
    ),
    (
        "c7 f8 00 10 00 00 90",
        &[(0, "xbegin 0x10002006"), (6, "nop")],
    ),
    ("c6 f8 05 90", &[(0, "xabort 0x5"), (3, "nop")]),
    (
        "0f a7 00 90 90",
        &[
            (0, "xstore-rng (bad)"),
            (1, "cmps   DWORD PTR ds:[esi],DWORD PTR es:[edi]"),
            (2, ".byte 0"),
            (3, "nop"),
            (4, "nop"),
        ],
    ),
    ("0f a7 c0 90", &[(0, "xstore-rng"), (3, "nop")]),
    ("0f a7 c8 90", &[(0, "xcrypt-ecb"), (3, "nop")]),
    ("8c f2 90", &[(0, "mov    edx,?"), (2, "nop")]),
    ("8c 36 90", &[(0, "mov    WORD PTR [esi],?"), (2, "nop")]),
    ("8e f0 90", &[(0, "mov    ?,eax"), (2, "nop")]),
    (
        "8e 36 90 90 90 90 90",
        &[
            (0, "mov    ?,WORD PTR [esi]"),
            (2, "nop"),
            (3, "nop"),
            (4, "nop"),
            (5, "nop"),
            (6, "nop"),
        ],
    ),
    (
        "8e 4e 01 90",
        &[(0, "mov    cs,WORD PTR [esi+0x1]"), (3, "nop")],
    ),
    ("0f 0b 90", &[(0, "ud2"), (2, "nop")]),
    ("d6 90", &[(0, "salc"), (1, "nop")]),
    ("d4 0a 90", &[(0, "aam    0xa"), (2, "nop")]),
    ("d5 0a 90", &[(0, "aad    0xa"), (2, "nop")]),
    (
        "f0 02 00 90",
        &[(0, "lock add al,BYTE PTR [eax]"), (3, "nop")],
    ),
    ("f0 40 90", &[(0, "lock inc eax"), (2, "nop")]),
    ("0f 20 d8 90", &[(0, "mov    eax,cr3"), (3, "nop")]),
    ("0f 22 d8 90", &[(0, "mov    cr3,eax"), (3, "nop")]),
    ("0f 21 c0 90", &[(0, "mov    eax,dr0"), (3, "nop")]),
    ("0f 23 c0 90", &[(0, "mov    dr0,eax"), (3, "nop")]),
    ("0f 20 08 90", &[(0, "mov    eax,cr1"), (3, "nop")]),
    (
        "0f 0f 90 90 90 90",
        &[
            (0, ".byte 0xf"),
            (1, ".byte 0xf"),
            (2, "nop"),
            (3, "nop"),
            (4, "nop"),
            (5, "nop"),
        ],
    ),
    ("d8 d1 90", &[(0, "fcom   st(1)"), (2, "nop")]),
    ("d8 d9 90", &[(0, "fcomp  st(1)"), (2, "nop")]),
    ("d9 c9 90", &[(0, "fxch   st(1)"), (2, "nop")]),
    ("dd e1 90", &[(0, "fucom  st(1)"), (2, "nop")]),
    ("dd e9 90", &[(0, "fucomp st(1)"), (2, "nop")]),
    ("df c1 90", &[(0, "ffreep st(1)"), (2, "nop")]),
    ("da e9 90", &[(0, "fucompp"), (2, "nop")]),
    ("db e2 90", &[(0, "fnclex"), (2, "nop")]),
    ("db e3 90", &[(0, "fninit"), (2, "nop")]),
    ("de d9 90", &[(0, "fcompp"), (2, "nop")]),
    ("d9 d0 90", &[(0, "fnop"), (2, "nop")]),
    ("9b d9 7d fc", &[(0, "fstcw  WORD PTR [ebp-0x4]")]),
    ("9b dd 7d f8", &[(0, "fstsw  WORD PTR [ebp-0x8]")]),
    ("9b 90", &[(0, "fwait"), (1, "nop")]),
    ("9b 9b 90", &[(0, "fwait"), (1, "fwait"), (2, "nop")]),
    (
        "9b 9b 9b d9 d0 90",
        &[(0, "fwait"), (1, "fnop"), (5, "nop")],
    ),
    ("9b d9 c0", &[(0, "fld    st(0)")]),
    ("9b db e2", &[(0, "fclex")]),
    ("9b db e3", &[(0, "finit")]),
    ("9b df e0", &[(0, "fstsw  ax")]),
    (
        "9b d9 7d fc 90",
        &[(0, "fstcw  WORD PTR [ebp-0x4]"), (4, "nop")],
    ),
    (
        "67 8b 00 90",
        &[(0, "mov    eax,DWORD PTR [bx+si]"), (3, "nop")],
    ),
    (
        "67 8b 40 05 90",
        &[(0, "mov    eax,DWORD PTR [bx+si+0x5]"), (4, "nop")],
    ),
    (
        "67 8b 80 00 10 90 90",
        &[
            (0, "mov    eax,DWORD PTR [bx+si+0x1000]"),
            (5, "nop"),
            (6, "nop"),
        ],
    ),
    (
        "67 8b 06 00 10 90 90",
        &[
            (0, "mov    eax,DWORD PTR ds:0x1000"),
            (5, "nop"),
            (6, "nop"),
        ],
    ),
    (
        "8b 04 05 00 10 00 00 90",
        &[(0, "mov    eax,DWORD PTR [eax*1+0x1000]"), (7, "nop")],
    ),
    (
        "8b 04 25 00 10 00 00 90",
        &[(0, "mov    eax,DWORD PTR [eiz*1+0x1000]"), (7, "nop")],
    ),
    (
        "8b 04 20 90",
        &[(0, "mov    eax,DWORD PTR [eax+eiz*1]"), (3, "nop")],
    ),
    (
        "8b 04 24 90",
        &[(0, "mov    eax,DWORD PTR [esp]"), (3, "nop")],
    ),
    (
        "8b 44 60 05 90",
        &[(0, "mov    eax,DWORD PTR [eax+eiz*2+0x5]"), (4, "nop")],
    ),
    (
        "8b 04 65 00 10 00 00 90",
        &[(0, "mov    eax,DWORD PTR [eiz*2+0x1000]"), (7, "nop")],
    ),
    ("8d 04 05 00 00 00 00", &[(0, "lea    eax,[eax*1+0x0]")]),
    (
        "8b 05 04 30 00 10",
        &[(0, "mov    eax,DWORD PTR ds:0x10003004")],
    ),
    (
        "8b 04 85 f0 ff ff ff",
        &[(0, "mov    eax,DWORD PTR [eax*4-0x10]")],
    ),
    ("8b 44 24 fc", &[(0, "mov    eax,DWORD PTR [esp-0x4]")]),
    (
        "8b 85 00 ff ff ff",
        &[(0, "mov    eax,DWORD PTR [ebp-0x100]")],
    ),
    (
        "8b 80 00 00 00 80",
        &[(0, "mov    eax,DWORD PTR [eax-0x80000000]")],
    ),
    (
        "8b 04 8d 00 00 00 80",
        &[(0, "mov    eax,DWORD PTR [ecx*4-0x80000000]")],
    ),
    ("8b 45 00", &[(0, "mov    eax,DWORD PTR [ebp+0x0]")]),
    ("8b 04 e4", &[(0, "mov    eax,DWORD PTR [esp+eiz*8]")]),
    (
        "8b 44 e4 10",
        &[(0, "mov    eax,DWORD PTR [esp+eiz*8+0x10]")],
    ),
    ("8d b4 26 00 00 00 00", &[(0, "lea    esi,[esi+eiz*1+0x0]")]),
    ("8d 74 26 00", &[(0, "lea    esi,[esi+eiz*1+0x0]")]),
    ("8d b6 00 00 00 00", &[(0, "lea    esi,[esi+0x0]")]),
    ("3e 8b 00", &[(0, "mov    eax,DWORD PTR ds:[eax]")]),
    ("8b 00", &[(0, "mov    eax,DWORD PTR [eax]")]),
    ("a0 04 30 00 10", &[(0, "mov    al,ds:0x10003004")]),
    ("a2 04 30 00 10", &[(0, "mov    ds:0x10003004,al")]),
    ("a3 04 30 00 10", &[(0, "mov    ds:0x10003004,eax")]),
    ("64 a1 18 00 00 00", &[(0, "mov    eax,fs:0x18")]),
    (
        "64 8b 0d 00 00 00 00",
        &[(0, "mov    ecx,DWORD PTR fs:0x0")],
    ),
    (
        "8b 0d f8 ff ff ff",
        &[(0, "mov    ecx,DWORD PTR ds:0xfffffff8")],
    ),
    (
        "ff 14 85 00 20 00 10",
        &[(0, "call   DWORD PTR [eax*4+0x10002000]")],
    ),
    (
        "ff 25 00 20 00 10",
        &[(0, "jmp    DWORD PTR ds:0x10002000")],
    ),
    ("ff 60 f0", &[(0, "jmp    DWORD PTR [eax-0x10]")]),
    (
        "c6 05 00 20 00 10 01",
        &[(0, "mov    BYTE PTR ds:0x10002000,0x1")],
    ),
    (
        "83 3d 00 20 00 10 00",
        &[(0, "cmp    DWORD PTR ds:0x10002000,0x0")],
    ),
    (
        "80 3d 00 20 00 10 00",
        &[(0, "cmp    BYTE PTR ds:0x10002000,0x0")],
    ),
    (
        "66 83 3d 00 20 00 10 00",
        &[(0, "cmp    WORD PTR ds:0x10002000,0x0")],
    ),
    ("8d 04 40", &[(0, "lea    eax,[eax+eax*2]")]),
    ("8d 04 80", &[(0, "lea    eax,[eax+eax*4]")]),
    ("8d 04 c0", &[(0, "lea    eax,[eax+eax*8]")]),
    ("8d 0c 41", &[(0, "lea    ecx,[ecx+eax*2]")]),
    ("8d 14 24", &[(0, "lea    edx,[esp]")]),
    ("8d 54 24 00", &[(0, "lea    edx,[esp+0x0]")]),
    (
        "f6 05 00 20 00 10 80",
        &[(0, "test   BYTE PTR ds:0x10002000,0x80")],
    ),
    ("eb fe", &[(0, "jmp    0x10001000")]),
    ("eb 00", &[(0, "jmp    0x10001002")]),
    ("e9 fb ff ff ff", &[(0, "jmp    0x10001000")]),
    ("e8 00 00 00 00", &[(0, "call   0x10001005")]),
    ("0f 85 fa ff ff ff", &[(0, "jne    0x10001000")]),
    ("74 fe", &[(0, "je     0x10001000")]),
    ("7f 02", &[(0, "jg     0x10001004")]),
    ("e3 05", &[(0, "jecxz  0x10001007")]),
    ("e2 fe", &[(0, "loop   0x10001000")]),
    ("e1 fe", &[(0, "loope  0x10001000")]),
    ("e0 fe", &[(0, "loopne 0x10001000")]),
    ("ff d0", &[(0, "call   eax")]),
    ("ff e0", &[(0, "jmp    eax")]),
    ("ff 10", &[(0, "call   DWORD PTR [eax]")]),
    ("ff 20", &[(0, "jmp    DWORD PTR [eax]")]),
    ("ff 30", &[(0, "push   DWORD PTR [eax]")]),
    ("ff 70 04", &[(0, "push   DWORD PTR [eax+0x4]")]),
    (
        "ff 35 00 20 00 10",
        &[(0, "push   DWORD PTR ds:0x10002000")],
    ),
    ("9a 00 10 00 10 23 00", &[(0, "call   0x23:0x10001000")]),
    ("ea 00 10 00 10 23 00", &[(0, "jmp    0x23:0x10001000")]),
    ("cd 21", &[(0, "int    0x21")]),
    ("cd 2e", &[(0, "int    0x2e")]),
    ("c2 04 00", &[(0, "ret    0x4")]),
    ("cb", &[(0, "retf")]),
    ("ca 08 00", &[(0, "retf   0x8")]),
    ("c8 10 00 00", &[(0, "enter  0x10,0x0")]),
    ("c8 00 00 01", &[(0, "enter  0x0,0x1")]),
    ("cf", &[(0, "iret")]),
    ("6a 00", &[(0, "push   0x0")]),
    ("6a 7f", &[(0, "push   0x7f")]),
    ("6a 80", &[(0, "push   0xffffff80")]),
    ("68 ff ff ff ff", &[(0, "push   0xffffffff")]),
    ("68 00 00 00 80", &[(0, "push   0x80000000")]),
    ("83 c0 80", &[(0, "add    eax,0xffffff80")]),
    ("83 e4 f0", &[(0, "and    esp,0xfffffff0")]),
    ("81 c4 00 ff ff ff", &[(0, "add    esp,0xffffff00")]),
    ("81 e1 ff ff ff 7f", &[(0, "and    ecx,0x7fffffff")]),
    ("b8 ff ff ff ff", &[(0, "mov    eax,0xffffffff")]),
    ("b0 ff", &[(0, "mov    al,0xff")]),
    ("b4 80", &[(0, "mov    ah,0x80")]),
    ("3c ff", &[(0, "cmp    al,0xff")]),
    ("24 7f", &[(0, "and    al,0x7f")]),
    ("a8 80", &[(0, "test   al,0x80")]),
    ("a9 00 00 00 80", &[(0, "test   eax,0x80000000")]),
    ("f6 c1 ff", &[(0, "test   cl,0xff")]),
    ("c1 e0 1f", &[(0, "shl    eax,0x1f")]),
    ("d1 e0", &[(0, "shl    eax,1")]),
    ("d1 e8", &[(0, "shr    eax,1")]),
    ("d1 f8", &[(0, "sar    eax,1")]),
    ("d1 d0", &[(0, "rcl    eax,1")]),
    ("d1 d8", &[(0, "rcr    eax,1")]),
    ("d1 c0", &[(0, "rol    eax,1")]),
    ("d1 c8", &[(0, "ror    eax,1")]),
    ("d0 e0", &[(0, "shl    al,1")]),
    ("d0 f0", &[(0, "shl    al,1")]),
    ("d1 f0", &[(0, "shl    eax,1")]),
    ("d3 e8", &[(0, "shr    eax,cl")]),
    ("d3 f8", &[(0, "sar    eax,cl")]),
    ("c0 e0 04", &[(0, "shl    al,0x4")]),
    ("0f a4 c1 04", &[(0, "shld   ecx,eax,0x4")]),
    ("0f a5 c1", &[(0, "shld   ecx,eax,cl")]),
    ("0f ac c1 04", &[(0, "shrd   ecx,eax,0x4")]),
    ("0f ad c1", &[(0, "shrd   ecx,eax,cl")]),
    ("6b c0 ff", &[(0, "imul   eax,eax,0xffffffff")]),
    ("69 c0 00 00 00 80", &[(0, "imul   eax,eax,0x80000000")]),
    ("e4 60", &[(0, "in     al,0x60")]),
    ("e5 60", &[(0, "in     eax,0x60")]),
    ("e6 61", &[(0, "out    0x61,al")]),
    ("e7 61", &[(0, "out    0x61,eax")]),
    ("ec", &[(0, "in     al,dx")]),
    ("ed", &[(0, "in     eax,dx")]),
    ("ee", &[(0, "out    dx,al")]),
    ("ef", &[(0, "out    dx,eax")]),
    ("6c", &[(0, "ins    BYTE PTR es:[edi],dx")]),
    ("6d", &[(0, "ins    DWORD PTR es:[edi],dx")]),
    ("6e", &[(0, "outs   dx,BYTE PTR ds:[esi]")]),
    ("6f", &[(0, "outs   dx,DWORD PTR ds:[esi]")]),
    (
        "d8 45 90 90 00",
        &[(0, "fadd   DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "d8 4d 90 90 00",
        &[(0, "fmul   DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "d8 55 90 90 00",
        &[(0, "fcom   DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "d8 5d 90 90 00",
        &[(0, "fcomp  DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "d8 65 90 90 00",
        &[(0, "fsub   DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "d8 6d 90 90 00",
        &[(0, "fsubr  DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "d8 75 90 90 00",
        &[(0, "fdiv   DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "d8 7d 90 90 00",
        &[(0, "fdivr  DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "d9 45 90 90 00",
        &[(0, "fld    DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    ("d9 4d 90 90 00", &[(0, "(bad) [ebp-0x70]"), (3, "nop")]),
    (
        "d9 55 90 90 00",
        &[(0, "fst    DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "d9 5d 90 90 00",
        &[(0, "fstp   DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    ("d9 65 90 90 00", &[(0, "fldenv [ebp-0x70]"), (3, "nop")]),
    (
        "d9 6d 90 90 00",
        &[(0, "fldcw  WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    ("d9 75 90 90 00", &[(0, "fnstenv [ebp-0x70]"), (3, "nop")]),
    (
        "d9 7d 90 90 00",
        &[(0, "fnstcw WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "da 45 90 90 00",
        &[(0, "fiadd  DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "da 4d 90 90 00",
        &[(0, "fimul  DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "da 55 90 90 00",
        &[(0, "ficom  DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "da 5d 90 90 00",
        &[(0, "ficomp DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "da 65 90 90 00",
        &[(0, "fisub  DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "da 6d 90 90 00",
        &[(0, "fisubr DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "da 75 90 90 00",
        &[(0, "fidiv  DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "da 7d 90 90 00",
        &[(0, "fidivr DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "db 45 90 90 00",
        &[(0, "fild   DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "db 4d 90 90 00",
        &[(0, "fisttp DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "db 55 90 90 00",
        &[(0, "fist   DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "db 5d 90 90 00",
        &[(0, "fistp  DWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    ("db 65 90 90 00", &[(0, "(bad) [ebp-0x70]"), (3, "nop")]),
    (
        "db 6d 90 90 00",
        &[(0, "fld    TBYTE PTR [ebp-0x70]"), (3, "nop")],
    ),
    ("db 75 90 90 00", &[(0, "(bad) [ebp-0x70]"), (3, "nop")]),
    (
        "db 7d 90 90 00",
        &[(0, "fstp   TBYTE PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dc 45 90 90 00",
        &[(0, "fadd   QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dc 4d 90 90 00",
        &[(0, "fmul   QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dc 55 90 90 00",
        &[(0, "fcom   QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dc 5d 90 90 00",
        &[(0, "fcomp  QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dc 65 90 90 00",
        &[(0, "fsub   QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dc 6d 90 90 00",
        &[(0, "fsubr  QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dc 75 90 90 00",
        &[(0, "fdiv   QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dc 7d 90 90 00",
        &[(0, "fdivr  QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dd 45 90 90 00",
        &[(0, "fld    QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dd 4d 90 90 00",
        &[(0, "fisttp QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dd 55 90 90 00",
        &[(0, "fst    QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "dd 5d 90 90 00",
        &[(0, "fstp   QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    ("dd 65 90 90 00", &[(0, "frstor [ebp-0x70]"), (3, "nop")]),
    ("dd 6d 90 90 00", &[(0, "(bad) [ebp-0x70]"), (3, "nop")]),
    ("dd 75 90 90 00", &[(0, "fnsave [ebp-0x70]"), (3, "nop")]),
    (
        "dd 7d 90 90 00",
        &[(0, "fnstsw WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "de 45 90 90 00",
        &[(0, "fiadd  WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "de 4d 90 90 00",
        &[(0, "fimul  WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "de 55 90 90 00",
        &[(0, "ficom  WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "de 5d 90 90 00",
        &[(0, "ficomp WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "de 65 90 90 00",
        &[(0, "fisub  WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "de 6d 90 90 00",
        &[(0, "fisubr WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "de 75 90 90 00",
        &[(0, "fidiv  WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "de 7d 90 90 00",
        &[(0, "fidivr WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "df 45 90 90 00",
        &[(0, "fild   WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "df 4d 90 90 00",
        &[(0, "fisttp WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "df 55 90 90 00",
        &[(0, "fist   WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "df 5d 90 90 00",
        &[(0, "fistp  WORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "df 65 90 90 00",
        &[(0, "fbld   TBYTE PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "df 6d 90 90 00",
        &[(0, "fild   QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "df 75 90 90 00",
        &[(0, "fbstp  TBYTE PTR [ebp-0x70]"), (3, "nop")],
    ),
    (
        "df 7d 90 90 00",
        &[(0, "fistp  QWORD PTR [ebp-0x70]"), (3, "nop")],
    ),
    ("d9 e0 90", &[(0, "fchs"), (2, "nop")]),
    ("d9 e1 90", &[(0, "fabs"), (2, "nop")]),
    ("d9 e4 90", &[(0, "ftst"), (2, "nop")]),
    ("d9 e5 90", &[(0, "fxam"), (2, "nop")]),
    ("d9 e8 90", &[(0, "fld1"), (2, "nop")]),
    ("d9 e9 90", &[(0, "fldl2t"), (2, "nop")]),
    ("d9 ea 90", &[(0, "fldl2e"), (2, "nop")]),
    ("d9 eb 90", &[(0, "fldpi"), (2, "nop")]),
    ("d9 ec 90", &[(0, "fldlg2"), (2, "nop")]),
    ("d9 ed 90", &[(0, "fldln2"), (2, "nop")]),
    ("d9 ee 90", &[(0, "fldz"), (2, "nop")]),
    ("d9 f0 90", &[(0, "f2xm1"), (2, "nop")]),
    ("d9 f1 90", &[(0, "fyl2x"), (2, "nop")]),
    ("d9 f2 90", &[(0, "fptan"), (2, "nop")]),
    ("d9 f3 90", &[(0, "fpatan"), (2, "nop")]),
    ("d9 f4 90", &[(0, "fxtract"), (2, "nop")]),
    ("d9 f5 90", &[(0, "fprem1"), (2, "nop")]),
    ("d9 f6 90", &[(0, "fdecstp"), (2, "nop")]),
    ("d9 f7 90", &[(0, "fincstp"), (2, "nop")]),
    ("d9 f8 90", &[(0, "fprem"), (2, "nop")]),
    ("d9 f9 90", &[(0, "fyl2xp1"), (2, "nop")]),
    ("d9 fa 90", &[(0, "fsqrt"), (2, "nop")]),
    ("d9 fb 90", &[(0, "fsincos"), (2, "nop")]),
    ("d9 fc 90", &[(0, "frndint"), (2, "nop")]),
    ("d9 fd 90", &[(0, "fscale"), (2, "nop")]),
    ("d9 fe 90", &[(0, "fsin"), (2, "nop")]),
    ("d9 ff 90", &[(0, "fcos"), (2, "nop")]),
    ("da e9 90", &[(0, "fucompp"), (2, "nop")]),
    ("db e2 90", &[(0, "fnclex"), (2, "nop")]),
    ("db e3 90", &[(0, "fninit"), (2, "nop")]),
    ("de d9 90", &[(0, "fcompp"), (2, "nop")]),
    ("df e0 90", &[(0, "fnstsw ax"), (2, "nop")]),
    ("d8 c0 90", &[(0, "fadd   st,st(0)"), (2, "nop")]),
    ("d8 c1 90", &[(0, "fadd   st,st(1)"), (2, "nop")]),
    ("d8 c8 90", &[(0, "fmul   st,st(0)"), (2, "nop")]),
    ("d8 e0 90", &[(0, "fsub   st,st(0)"), (2, "nop")]),
    ("d8 e8 90", &[(0, "fsubr  st,st(0)"), (2, "nop")]),
    ("d8 f0 90", &[(0, "fdiv   st,st(0)"), (2, "nop")]),
    ("d8 f8 90", &[(0, "fdivr  st,st(0)"), (2, "nop")]),
    ("dc c0 90", &[(0, "fadd   st(0),st"), (2, "nop")]),
    ("dc c8 90", &[(0, "fmul   st(0),st"), (2, "nop")]),
    ("dc e0 90", &[(0, "fsubr  st(0),st"), (2, "nop")]),
    ("dc e8 90", &[(0, "fsub   st(0),st"), (2, "nop")]),
    ("dc f0 90", &[(0, "fdivr  st(0),st"), (2, "nop")]),
    ("dc f8 90", &[(0, "fdiv   st(0),st"), (2, "nop")]),
    ("de c0 90", &[(0, "faddp  st(0),st"), (2, "nop")]),
    ("de c8 90", &[(0, "fmulp  st(0),st"), (2, "nop")]),
    ("de e0 90", &[(0, "fsubrp st(0),st"), (2, "nop")]),
    ("de e8 90", &[(0, "fsubp  st(0),st"), (2, "nop")]),
    ("de f0 90", &[(0, "fdivrp st(0),st"), (2, "nop")]),
    ("de f8 90", &[(0, "fdivp  st(0),st"), (2, "nop")]),
    ("da c1 90", &[(0, "fcmovb st,st(1)"), (2, "nop")]),
    ("da c9 90", &[(0, "fcmove st,st(1)"), (2, "nop")]),
    ("da d1 90", &[(0, "fcmovbe st,st(1)"), (2, "nop")]),
    ("da d9 90", &[(0, "fcmovu st,st(1)"), (2, "nop")]),
    ("db c1 90", &[(0, "fcmovnb st,st(1)"), (2, "nop")]),
    ("db c9 90", &[(0, "fcmovne st,st(1)"), (2, "nop")]),
    ("db d1 90", &[(0, "fcmovnbe st,st(1)"), (2, "nop")]),
    ("db d9 90", &[(0, "fcmovnu st,st(1)"), (2, "nop")]),
    ("db e9 90", &[(0, "fucomi st,st(1)"), (2, "nop")]),
    ("db f1 90", &[(0, "fcomi  st,st(1)"), (2, "nop")]),
    ("df e9 90", &[(0, "fucomip st,st(1)"), (2, "nop")]),
    ("df f1 90", &[(0, "fcomip st,st(1)"), (2, "nop")]),
    ("d9 c0 90", &[(0, "fld    st(0)"), (2, "nop")]),
    ("d9 c9 90", &[(0, "fxch   st(1)"), (2, "nop")]),
    ("dd c1 90", &[(0, "ffree  st(1)"), (2, "nop")]),
    ("dd d1 90", &[(0, "fst    st(1)"), (2, "nop")]),
    ("dd d9 90", &[(0, "fstp   st(1)"), (2, "nop")]),
    ("dd e1 90", &[(0, "fucom  st(1)"), (2, "nop")]),
    ("dd e9 90", &[(0, "fucomp st(1)"), (2, "nop")]),
    ("df c1 90", &[(0, "ffreep st(1)"), (2, "nop")]),
    ("d9 d0 90", &[(0, "fnop"), (2, "nop")]),
    ("a4", &[(0, "movs   BYTE PTR es:[edi],BYTE PTR ds:[esi]")]),
    ("a5", &[(0, "movs   DWORD PTR es:[edi],DWORD PTR ds:[esi]")]),
    ("a6", &[(0, "cmps   BYTE PTR ds:[esi],BYTE PTR es:[edi]")]),
    ("a7", &[(0, "cmps   DWORD PTR ds:[esi],DWORD PTR es:[edi]")]),
    ("aa", &[(0, "stos   BYTE PTR es:[edi],al")]),
    ("ab", &[(0, "stos   DWORD PTR es:[edi],eax")]),
    ("ac", &[(0, "lods   al,BYTE PTR ds:[esi]")]),
    ("ad", &[(0, "lods   eax,DWORD PTR ds:[esi]")]),
    ("ae", &[(0, "scas   al,BYTE PTR es:[edi]")]),
    ("af", &[(0, "scas   eax,DWORD PTR es:[edi]")]),
    (
        "66 a7",
        &[(0, "cmps   WORD PTR ds:[esi],WORD PTR es:[edi]")],
    ),
    ("66 af", &[(0, "scas   ax,WORD PTR es:[edi]")]),
    ("d7", &[(0, "xlat   BYTE PTR ds:[ebx]")]),
    ("9e", &[(0, "sahf")]),
    ("9f", &[(0, "lahf")]),
    ("f8", &[(0, "clc")]),
    ("f9", &[(0, "stc")]),
    ("fa", &[(0, "cli")]),
    ("fb", &[(0, "sti")]),
    ("fc", &[(0, "cld")]),
    ("fd", &[(0, "std")]),
    ("0f 01 10", &[(0, "lgdtd  [eax]")]),
    ("0f 01 18", &[(0, "lidtd  [eax]")]),
    ("0f 01 00", &[(0, "sgdtd  [eax]")]),
    ("0f 01 08", &[(0, "sidtd  [eax]")]),
    ("0f 01 20", &[(0, "smsw   WORD PTR [eax]")]),
    ("0f 01 30", &[(0, "lmsw   WORD PTR [eax]")]),
    ("0f 01 38", &[(0, "invlpg BYTE PTR [eax]")]),
    ("0f 00 00", &[(0, "sldt   WORD PTR [eax]")]),
    ("0f 00 08", &[(0, "str    WORD PTR [eax]")]),
    ("0f 18 00", &[(0, "prefetchnta BYTE PTR [eax]")]),
    ("0f 0d 00", &[(0, "prefetch BYTE PTR [eax]")]),
    ("c4 00", &[(0, "les    eax,FWORD PTR [eax]")]),
    ("62 00", &[(0, "bound  eax,QWORD PTR [eax]")]),
    ("63 c0", &[(0, "arpl   eax,eax")]),
    ("63 00", &[(0, "arpl   WORD PTR [eax],eax")]),
    ("0f b2 00", &[(0, "lss    eax,FWORD PTR [eax]")]),
    ("0f b4 00", &[(0, "lfs    eax,FWORD PTR [eax]")]),
    ("0f b5 00", &[(0, "lgs    eax,FWORD PTR [eax]")]),
    ("0f c7 08", &[(0, "cmpxchg8b QWORD PTR [eax]")]),
    ("0f ae 00", &[(0, "fxsave [eax]")]),
    ("0f ae e8", &[(0, "lfence")]),
    ("0f ae f0", &[(0, "mfence")]),
    ("0f ae f8", &[(0, "sfence")]),
    ("0f 77", &[(0, "emms")]),
    ("0f 6f 00", &[(0, "movq   mm0,QWORD PTR [eax]")]),
    ("0f 7f 00", &[(0, "movq   QWORD PTR [eax],mm0")]),
    ("66 0f 6f 00", &[(0, "movdqa xmm0,XMMWORD PTR [eax]")]),
    ("f3 0f 6f 00", &[(0, "movdqu xmm0,XMMWORD PTR [eax]")]),
    ("0f 28 00", &[(0, "movaps xmm0,XMMWORD PTR [eax]")]),
    ("0f 10 00", &[(0, "movups xmm0,XMMWORD PTR [eax]")]),
    ("f3 0f 10 00", &[(0, "movss  xmm0,DWORD PTR [eax]")]),
    ("f2 0f 10 00", &[(0, "movsd  xmm0,QWORD PTR [eax]")]),
    ("66 0f 10 00", &[(0, "movupd xmm0,XMMWORD PTR [eax]")]),
    ("0f 5a c0", &[(0, "cvtps2pd xmm0,xmm0")]),
    ("0f 2a c0", &[(0, "cvtpi2ps xmm0,mm0")]),
    ("f3 0f 2a 00", &[(0, "cvtsi2ss xmm0,DWORD PTR [eax]")]),
    ("f2 0f 2c c0", &[(0, "cvttsd2si eax,xmm0")]),
    ("0f c6 c0 01", &[(0, "shufps xmm0,xmm0,0x1")]),
    ("0f 70 c0 01", &[(0, "pshufw mm0,mm0,0x1")]),
    ("66 0f 70 c0 01", &[(0, "pshufd xmm0,xmm0,0x1")]),
    ("0f d6 00", &[(0, "(bad)")]),
    ("0f 7e c0", &[(0, "movd   eax,mm0")]),
    ("66 0f 7e c0", &[(0, "movd   eax,xmm0")]),
    ("0f 6e c0", &[(0, "movd   mm0,eax")]),
    ("66 0f 6e c0", &[(0, "movd   xmm0,eax")]),
    ("f3 0f 7e 00", &[(0, "movq   xmm0,QWORD PTR [eax]")]),
    ("0f 12 00", &[(0, "movlps xmm0,QWORD PTR [eax]")]),
    ("0f 16 00", &[(0, "movhps xmm0,QWORD PTR [eax]")]),
    ("0f 50 c0", &[(0, "movmskps eax,xmm0")]),
    ("0f c5 c0 01", &[(0, "pextrw eax,mm0,0x1")]),
    ("0f c3 00", &[(0, "movnti DWORD PTR [eax],eax")]),
    ("0f 0d 08", &[(0, "prefetchw BYTE PTR [eax]")]),
    ("0f 18 08", &[(0, "prefetcht0 BYTE PTR [eax]")]),
    ("0f 18 10", &[(0, "prefetcht1 BYTE PTR [eax]")]),
    ("0f 18 18", &[(0, "prefetcht2 BYTE PTR [eax]")]),
    ("0f 1f 40 00", &[(0, "nop    DWORD PTR [eax+0x0]")]),
    ("0f 1f 44 00 00", &[(0, "nop    DWORD PTR [eax+eax*1+0x0]")]),
    (
        "66 0f 1f 44 00 00",
        &[(0, "nop    WORD PTR [eax+eax*1+0x0]")],
    ),
    ("0f 1f 80 00 00 00 00", &[(0, "nop    DWORD PTR [eax+0x0]")]),
    (
        "66 2e 0f 1f 84 00 00 00 00 00",
        &[(0, "nop    WORD PTR cs:[eax+eax*1+0x0]")],
    ),
    ("f3 90", &[(0, "pause")]),
    ("66 90", &[(0, "xchg   ax,ax")]),
    ("90", &[(0, "nop")]),
    ("f4", &[(0, "hlt")]),
    ("cc", &[(0, "int3")]),
    ("cd 03", &[(0, "int    0x3")]),
    ("ce", &[(0, "into")]),
    ("f1", &[(0, "int1")]),
    ("0f 05", &[(0, "syscall")]),
    ("0f 34", &[(0, "sysenter")]),
    ("0f 35", &[(0, "sysexit")]),
    ("0f 31", &[(0, "rdtsc")]),
    ("0f a2", &[(0, "cpuid")]),
    ("0f 30", &[(0, "wrmsr")]),
    ("0f 32", &[(0, "rdmsr")]),
    ("0f 33", &[(0, "rdpmc")]),
    ("0f 08", &[(0, "invd")]),
    ("0f 09", &[(0, "wbinvd")]),
    ("0f 06", &[(0, "clts")]),
    ("0f 07", &[(0, "sysret")]),
    ("0f 0b", &[(0, "ud2")]),
    ("c8 05 00 00", &[(0, "enter  0x5,0x0")]),
    ("0f ba e0 03", &[(0, "bt     eax,0x3")]),
    ("0f ba 20 03", &[(0, "bt     DWORD PTR [eax],0x3")]),
    ("0f a3 c8", &[(0, "bt     eax,ecx")]),
    ("0f bc c1", &[(0, "bsf    eax,ecx")]),
    ("0f bd c1", &[(0, "bsr    eax,ecx")]),
    ("f3 0f bc c1", &[(0, "tzcnt  eax,ecx")]),
    ("f3 0f bd c1", &[(0, "lzcnt  eax,ecx")]),
    ("0f b6 00", &[(0, "movzx  eax,BYTE PTR [eax]")]),
    ("0f b7 00", &[(0, "movzx  eax,WORD PTR [eax]")]),
    ("0f be c0", &[(0, "movsx  eax,al")]),
    ("0f bf 00", &[(0, "movsx  eax,WORD PTR [eax]")]),
    ("0f af 00", &[(0, "imul   eax,DWORD PTR [eax]")]),
    ("0f b0 08", &[(0, "cmpxchg BYTE PTR [eax],cl")]),
    ("0f b1 08", &[(0, "cmpxchg DWORD PTR [eax],ecx")]),
    ("0f c0 08", &[(0, "xadd   BYTE PTR [eax],cl")]),
    ("0f c1 08", &[(0, "xadd   DWORD PTR [eax],ecx")]),
    ("f0 0f c1 08", &[(0, "lock xadd DWORD PTR [eax],ecx")]),
    ("f0 0f b1 08", &[(0, "lock cmpxchg DWORD PTR [eax],ecx")]),
    ("87 00", &[(0, "xchg   DWORD PTR [eax],eax")]),
    ("86 00", &[(0, "xchg   BYTE PTR [eax],al")]),
    ("91", &[(0, "xchg   ecx,eax")]),
    ("92", &[(0, "xchg   edx,eax")]),
    ("98", &[(0, "cwde")]),
    ("99", &[(0, "cdq")]),
    ("9b", &[(0, "fwait")]),
    ("9c", &[(0, "pushf")]),
    ("9d", &[(0, "popf")]),
    ("60", &[(0, "pusha")]),
    ("61", &[(0, "popa")]),
    ("c9", &[(0, "leave")]),
    ("c3", &[(0, "ret")]),
    ("0f 90 c0", &[(0, "seto   al")]),
    ("0f 94 c0", &[(0, "sete   al")]),
    ("0f 9f 00", &[(0, "setg   BYTE PTR [eax]")]),
    ("0f 40 c1", &[(0, "cmovo  eax,ecx")]),
    ("0f 44 c1", &[(0, "cmove  eax,ecx")]),
    ("0f 4f c1", &[(0, "cmovg  eax,ecx")]),
    ("0f c8", &[(0, "bswap  eax")]),
    ("0f cf", &[(0, "bswap  edi")]),
    ("0f 02 00", &[(0, "lar    eax,WORD PTR [eax]")]),
    ("0f 03 00", &[(0, "lsl    eax,WORD PTR [eax]")]),
    ("a8 01", &[(0, "test   al,0x1")]),
    ("f7 d8", &[(0, "neg    eax")]),
    ("f7 f1", &[(0, "div    ecx")]),
    ("f7 f9", &[(0, "idiv   ecx")]),
    ("f7 e1", &[(0, "mul    ecx")]),
    ("f7 e9", &[(0, "imul   ecx")]),
    ("f6 d0", &[(0, "not    al")]),
    ("f6 f1", &[(0, "div    cl")]),
    ("f7 c1 ff 00 00 00", &[(0, "test   ecx,0xff")]),
    ("f7 00 ff 00 00 00", &[(0, "test   DWORD PTR [eax],0xff")]),
    ("fe c0", &[(0, "inc    al")]),
    ("fe c8", &[(0, "dec    al")]),
    ("ff c0", &[(0, "inc    eax")]),
    ("ff c8", &[(0, "dec    eax")]),
    ("ff 00", &[(0, "inc    DWORD PTR [eax]")]),
    ("ff 08", &[(0, "dec    DWORD PTR [eax]")]),
    ("40", &[(0, "inc    eax")]),
    ("48", &[(0, "dec    eax")]),
    ("4f", &[(0, "dec    edi")]),
    ("50", &[(0, "push   eax")]),
    ("5f", &[(0, "pop    edi")]),
    ("8f 00", &[(0, "pop    DWORD PTR [eax]")]),
    ("8f c0", &[(0, "pop    eax")]),
    ("07", &[(0, "pop    es")]),
    ("17", &[(0, "pop    ss")]),
    ("1f", &[(0, "pop    ds")]),
    ("06", &[(0, "push   es")]),
    ("0e", &[(0, "push   cs")]),
    ("16", &[(0, "push   ss")]),
    ("1e", &[(0, "push   ds")]),
    ("0f a0", &[(0, "push   fs")]),
    ("0f a1", &[(0, "pop    fs")]),
    ("0f a8", &[(0, "push   gs")]),
    ("0f a9", &[(0, "pop    gs")]),
    ("27", &[(0, "daa")]),
    ("2f", &[(0, "das")]),
    ("37", &[(0, "aaa")]),
    ("3f", &[(0, "aas")]),
    ("9e", &[(0, "sahf")]),
    ("0f 0f 00 90", &[(0, "pfcmpge mm0,QWORD PTR [eax]")]),
    ("9b d9", &[(0, "fwait"), (1, ".byte 0xd9")]),
    ("9b d9 7d", &[(0, "fwait"), (1, ".byte 0xd9"), (2, ".byte 0x7d")]),
    ("9b 9b d9", &[(0, "fwait"), (1, "fwait"), (2, ".byte 0xd9")]),
    ("9b d9 05 00 00", &[(0, "fwait"), (1, ".byte 0xd9"), (2, ".byte 0x5")]),
    ("9b 66 d9 7d fc 90", &[(0, "data16 fstcw WORD PTR [ebp-0x4]"), (5, "nop")]),
    ("66 9b d9 7d fc 90", &[(0, "data16 fstcw WORD PTR [ebp-0x4]"), (5, "nop")]),
    ("9b f0 d9 7d fc 90", &[(0, "lock fstcw WORD PTR [ebp-0x4]"), (5, "nop")]),
    ("9b 66 d9 d0 90", &[(0, "data16 fnop"), (4, "nop")]),
    ("66 9b 90", &[(0, "data16 fwait"), (2, "nop")]),
    ("9b 66 90", &[(0, "fwait"), (1, "xchg   ax,ax")]),
    ("9b 26 65 9b 38 b5 f1 55", &[(0, "es gs fwait"), (3, "fwait"), (4, ".byte 0x38"), (5, "mov    ch,0xf1"), (7, "push   ebp")]),
    ("65 9b 2e de 93 33 b8 8c c7 6f", &[(0, "gs fwait"), (2, "ficom  WORD PTR cs:[ebx-0x387347cd]"), (9, "outs   dx,DWORD PTR ds:[esi]")]),
    ("f2 9b 2e da 17 2a fb fc", &[(0, "repnz fwait"), (2, "ficom  DWORD PTR cs:[edi]"), (5, "sub    bh,bl"), (7, "cld")]),
    ("9b 64 f0 9b b3 a7 c1 c5", &[(0, "fs lock fwait"), (3, "fwait"), (4, "mov    bl,0xa7"), (6, ".byte 0xc1"), (7, ".byte 0xc5")]),
    ("9b 66 9b 90 90", &[(0, "data16 fwait"), (2, "fwait"), (3, "nop"), (4, "nop")]),
    ("9b 9b 9b d9 7d fc 90", &[(0, "fwait"), (1, "fstcw  WORD PTR [ebp-0x4]"), (6, "nop")]),
    ("9b 9b d9 7d fc 90", &[(0, "fstcw  WORD PTR [ebp-0x4]"), (5, "nop")]),
    ("0f b2 c0 90 90", &[(0, "lss    eax,(bad)"), (1, "mov    dl,0xc0"), (3, "nop"), (4, "nop")]),
    ("0f b4 c0 90 90", &[(0, "lfs    eax,(bad)"), (1, "mov    ah,0xc0"), (3, "nop"), (4, "nop")]),
    ("0f b5 c0 90 90", &[(0, "lgs    eax,(bad)"), (1, "mov    ch,0xc0"), (3, "nop"), (4, "nop")]),
    ("0f c3 c0 90 90", &[(0, "movnti (bad),eax"), (1, "ret"), (2, ".byte 0xc0"), (3, "nop"), (4, "nop")]),
    ("0f 2b c0 90 90", &[(0, "movntps (bad),xmm0"), (1, "sub    eax,eax"), (3, "nop"), (4, "nop")]),
    ("66 0f 2b c0 90 90", &[(0, "movntpd (bad),xmm0"), (2, "sub    eax,eax"), (4, "nop"), (5, "nop")]),
    ("0f e7 c0 90 90", &[(0, "movntq (bad),mm0"), (1, "out    0xc0,eax"), (3, "nop"), (4, "nop")]),
    ("66 0f e7 c0 90 90", &[(0, "movntdq (bad),xmm0"), (2, "out    0xc0,eax"), (4, "nop"), (5, "nop")]),
    ("0f 13 c0 90 90", &[(0, "movlps (bad),xmm0"), (1, "adc    eax,eax"), (3, "nop"), (4, "nop")]),
    ("0f c7 c8 90 90", &[(0, "cmpxchg8b (bad)"), (1, "(bad)"), (2, ".byte 0xc8"), (3, "nop"), (4, "nop")]),
    ("0f 38 f0 c0 90 90", &[(0, "movbe  eax,(bad)"), (1, "cmp    al,dh"), (3, ".byte 0xc0"), (4, "nop"), (5, "nop")]),
    ("0f 0d c8 90 90", &[(0, "prefetchw (bad)"), (1, ".byte 0xd"), (2, ".byte 0xc8"), (3, "nop"), (4, "nop")]),
    ("0f 50 08 90 90", &[(0, "movmskps ecx,(bad)"), (1, "push   eax"), (2, ".byte 0x8"), (3, "nop"), (4, "nop")]),
    ("0f d7 30 90 90", &[(0, "pmovmskb esi,(bad)"), (1, "xlat   BYTE PTR ds:[ebx]"), (2, ".byte 0x30"), (3, "nop"), (4, "nop")]),
    ("0f f7 05 90 90 90 90", &[(0, "maskmovq mm0,(bad)"), (1, ".byte 0xf7"), (2, "add    eax,0x90909090")]),
    ("0f 0f c0 ff 90 90", &[(0, "(bad)"), (1, "xadd   bh,bh"), (4, "nop"), (5, "nop")]),
    ("0f 0f c1 0d 90 90", &[(0, "pi2fd  mm0,mm1"), (4, "nop"), (5, "nop")]),
    ("0f a6 08 90 90", &[(0, "xsha1  (bad)"), (1, "cmps   BYTE PTR ds:[esi],BYTE PTR es:[edi]"), (2, ".byte 0x8"), (3, "nop"), (4, "nop")]),
    ("0f a6 c0 90 90", &[(0, "(bad)"), (2, ".byte 0xc0"), (3, "nop"), (4, "nop")]),
    ("0f a6 c8 90 90", &[(0, "xsha1"), (3, "nop"), (4, "nop")]),
    ("0f a6 f0 90 90", &[(0, "montmul2"), (3, "nop"), (4, "nop")]),
    ("0f a7 00 90 90", &[(0, "xstore-rng (bad)"), (1, "cmps   DWORD PTR ds:[esi],DWORD PTR es:[edi]"), (2, ".byte 0"), (3, "nop"), (4, "nop")]),
    ("0f a7 30 90 90", &[(0, "(bad)"), (2, ".byte 0x30"), (3, "nop"), (4, "nop")]),
    ("0f a7 f8 90 90", &[(0, "xrng2"), (3, "nop"), (4, "nop")]),
    ("f2 0f a6 c0 90 90", &[(0, "sm2"), (4, "nop"), (5, "nop")]),
    ("f2 0f a6 00 90 90", &[(0, "sm2    (bad)"), (2, "cmps   BYTE PTR ds:[esi],BYTE PTR es:[edi]"), (3, ".byte 0"), (4, "nop"), (5, "nop")]),
    ("62 e9 c6 90 90 90 90 90", &[(0, "(bad) {bad},{rn-bad}"), (3, "nop"), (4, "nop"), (5, "nop"), (6, "nop"), (7, "nop")]),
    ("62 df f7 90 90 90 90 90", &[(0, "(bad) {bad},{rn-bad}"), (3, "nop"), (4, "nop"), (5, "nop"), (6, "nop"), (7, "nop")]),
    ("62 e6 b1 90 90 90 90 90", &[(0, "(bad) {bad},{rn-bad}"), (3, "nop"), (4, "nop"), (5, "nop"), (6, "nop"), (7, "nop")]),
    ("62 f5 79 90 90 90 90 90", &[(0, "(bad) {bad},{rn-bad}"), (3, "nop"), (4, "nop"), (5, "nop"), (6, "nop"), (7, "nop")]),
    ("62 f1 7c 08 ff c0 90 90", &[(0, "(bad)"), (5, ".byte 0xc0"), (6, "nop"), (7, "nop")]),
    ("62 f2 7c 08 00 c0 90 90", &[(0, "(bad)"), (5, ".byte 0xc0"), (6, "nop"), (7, "nop")]),
    ("62 f1 7c 48 58 c1 90 90", &[(0, "vaddps zmm0,zmm0,zmm1"), (6, "nop"), (7, "nop")]),
    ("62 f1 7c 08 58 00 90 90", &[(0, "{evex} vaddps xmm0,xmm0,XMMWORD PTR [eax]"), (6, "nop"), (7, "nop")]),
    ("62 f1 7c 68 58 c1 90 90", &[(0, "(bad)"), (5, ".byte 0xc1"), (6, "nop"), (7, "nop")]),
    ("62 c0 90 90 90 90", &[(0, "(bad)"), (1, ".byte 0xc0"), (2, "nop"), (3, "nop"), (4, "nop"), (5, "nop")]),
    ("62 f1 7c 08 c6 c1 05 90", &[(0, "{evex} vshufps xmm0,xmm0,xmm1,0x5"), (7, "nop")]),
    ("c5 f8 77 90", &[(0, "vzeroupper"), (3, "nop")]),
    ("c5 fc 77 90", &[(0, "vzeroall"), (3, "nop")]),
    ("c5 e1 73 90 63 a0 5f e2 90", &[(0, "vpsrlq xmm3,(bad),0x90"), (4, ".byte 0x63"), (5, ".byte 0xa0"), (6, "pop    edi"), (7, "loop   0x10000f99")]),
    ("c5 f8 73 90 01 90 90", &[(0, "(bad)"), (3, "nop"), (4, ".byte 0x1"), (5, "nop"), (6, "nop")]),
    ("c5 f0 90 c0 90 90 90", &[(0, "(bad)"), (3, ".byte 0xc0"), (4, "nop"), (5, "nop"), (6, "nop")]),
    ("c5 f8 90 c0 90 90 90", &[(0, "kmovw  k0,k0"), (4, "nop"), (5, "nop"), (6, "nop")]),
    ("c4 e2 7f f6 a1 43 b8 a8 ff 8b 90", &[(0, "(bad)"), (4, "mov    eax,ds:0xffa8b843"), (9, ".byte 0x8b"), (10, "nop")]),
    ("c5 f9 6f c1 90", &[(0, "vmovdqa xmm0,xmm1"), (4, "nop")]),
    ("0f 24 c0 90 90", &[(0, "mov    eax,tr0"), (3, "nop"), (4, "nop")]),
    ("0f 26 08 90 90", &[(0, "mov    tr1,eax"), (3, "nop"), (4, "nop")]),
    ("0f 24 b5 90 90", &[(0, "mov    ebp,tr6"), (3, "nop"), (4, "nop")]),
    ("0f 25 c0 90 90", &[(0, "(bad)"), (2, ".byte 0xc0"), (3, "nop"), (4, "nop")]),
    ("3e 3e 3e 3e 3e 81 84 01 02 03 04 05 06 07 08 09 90", &[(0, "ds ds ds ds (bad)"), (15, ".byte 0x9"), (16, "nop")]),
    ("3e 3e 3e 3e 81 84 01 02 03 04 05 06 07 08 09 90", &[(0, "ds ds ds add DWORD PTR ds:[ecx+eax*1+0x5040302],0x9080706"), (15, "nop")]),
    ("66 66 66 66 66 66 66 66 66 66 66 66 66 66 90 90", &[(0, "data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16"), (14, "nop"), (15, "nop")]),
    ("66 66 66 66 66 66 66 66 66 66 66 66 66 66 66 90 90", &[(0, "data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16"), (14, "xchg   ax,ax"), (16, "nop")]),
    ("66 66 66 66 66 66 66 66 66 66 66 66 66 66 66 66 90 90", &[(0, "data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16 data16"), (14, "data16 xchg ax,ax"), (17, "nop")]),
    ("f0 f0 f0 f0 f0 f0 f0 f0 f0 f0 f0 f0 f0 f0 f0 f0 01 00 90", &[(0, "lock lock lock lock lock lock lock lock lock lock lock lock lock lock"), (14, "lock lock add DWORD PTR [eax],eax"), (18, "nop")]),
    ("3e 3e 3e 3e 3e 3e 3e 3e 3e 3e 3e 3e 3e 8b 00 90", &[(0, "ds ds ds ds ds ds ds ds ds ds ds ds mov eax,DWORD PTR ds:[eax]"), (15, "nop")]),
    ("3e 3e 3e 3e 3e 3e 3e 3e 3e 3e 3e 3e 3e 3e 8b 00 90 90", &[(0, "ds ds ds ds ds ds ds ds ds ds ds ds ds ds"), (14, "mov    eax,DWORD PTR [eax]"), (16, "nop"), (17, "nop")]),
    ("66 0f ae 00 90", &[(0, "data16 fxsave [eax]"), (4, "nop")]),
    ("f3 0f ae 00 90", &[(0, "repz fxsave [eax]"), (4, "nop")]),
    ("66 0f c7 3e f1 43", &[(0, "data16 vmptrst QWORD PTR [esi]"), (4, "int1"), (5, "inc    ebx")]),
    ("66 0f 01 c0 90", &[(0, "data16 enclv"), (4, "nop")]),
    ("0f 01 f8 90", &[(0, "swapgs"), (3, "nop")]),
    ("66 c7 78 da bd 16 ff 89", &[(0, "xbeginw (bad),0xea7c"), (4, ".byte 0xbd"), (5, "push   ss"), (6, ".byte 0xff"), (7, ".byte 0x89")]),
    ("66 c7 f8 da bd 16 ff 89", &[(0, "xbeginw 0xcddf"), (5, "push   ss"), (6, ".byte 0xff"), (7, ".byte 0x89")]),
    ("66 0f 78 f5 43 52 38 0a", &[(0, "extrq  xmm5,0x43,0x52"), (6, "cmp    BYTE PTR [edx],cl")]),
    ("f2 0f 78 f5 43 52 38 0a", &[(0, "insertq xmm6,xmm5,0x43,0x52"), (6, "cmp    BYTE PTR [edx],cl")]),
    ("66 0f 78 00 90 90 90", &[(0, "extrq  (bad),0x78,0x0"), (4, "nop"), (5, "nop"), (6, "nop")]),
    ("67 0f 1a 69 9e 95 90", &[(0, "addr16 bndldx (bad),(bad)"), (4, "sahf"), (5, "xchg   ebp,eax"), (6, "nop")]),
    ("67 0f 1a 00 90 90 90", &[(0, "addr16 bndldx bnd0,(bad)"), (4, "nop"), (5, "nop"), (6, "nop")]),
    ("db e0", &[(0, "fneni(8087 only)")]),
    ("db e1", &[(0, "fndisi(8087 only)")]),
    ("db e4", &[(0, "fnsetpm(287 only)")]),
    ("db e5", &[(0, "frstpm(287 only)")]),
    ("9b db e0 90", &[(0, "feni(8087 only)"), (3, "nop")]),
];

#[test]
fn hand_assembled_sequences_list_as_the_reference_tool_listed_them() {
    let mut failures = Vec::new();
    for (bytes, want) in RULES {
        let mut got = Vec::new();
        sta_mission_convert::disasm::disassemble_bytes(0x1000_1000, &hex(bytes), &mut got);
        let got: Vec<(u32, &str)> = got
            .iter()
            .map(|(a, t)| (a - 0x1000_1000, t.as_str()))
            .collect();
        if got != *want {
            failures.push(format!("{bytes}: want {want:?}, got {got:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn section(
    name: &str,
    va: u32,
    vsize: u32,
    raw_off: u32,
    raw_size: u32,
    flags: u32,
) -> sta_mission_convert::Section {
    sta_mission_convert::Section {
        name: name.into(),
        va,
        vsize,
        raw_off,
        raw_size,
        characteristics: flags,
    }
}

const CODE_FLAGS: u32 = 0x6000_0020;

/// A flat image whose bytes are indexed by offset from the base; every section's data sits at
/// its own virtual address.
fn flat(
    sections: Vec<sta_mission_convert::Section>,
    put: &[(usize, &[u8])],
) -> sta_mission_convert::FlatImage {
    let mut img = sta_mission_convert::FlatImage::new(0x1000_0000, 0x400);
    for (o, b) in put {
        img.data[*o..*o + b.len()].copy_from_slice(b);
    }
    img.sections = sections;
    img
}

#[test]
fn sections_marked_as_code_are_swept_in_section_order() {
    // push ebp; ret in each section; the data section holds the same bytes and is not swept.
    let img = flat(
        vec![
            section(".a", 0x100, 2, 0x100, 2, CODE_FLAGS),
            section(".d", 0x120, 2, 0x120, 2, 0x4000_0040),
            section(".b", 0x140, 2, 0x140, 2, 0x0000_0020),
            section(".c", 0x160, 2, 0x160, 2, 0x2000_0000),
            section(".e", 0x180, 2, 0x180, 0, CODE_FLAGS),
        ],
        &[
            (0x100, &[0x55, 0xc3]),
            (0x120, &[0x55, 0xc3]),
            (0x140, &[0x55, 0xc3]),
            (0x160, &[0x55, 0xc3]),
            (0x180, &[0x55, 0xc3]),
        ],
    );
    let l = sta_mission_convert::disasm::Listing::from_image(&img).unwrap();
    let got: Vec<(u32, &str)> = l.ins.iter().map(|(a, t)| (*a, t.as_str())).collect();
    assert_eq!(
        got,
        [
            (0x1000_0100, "push   ebp"),
            (0x1000_0101, "ret"),
            (0x1000_0140, "push   ebp"),
            (0x1000_0141, "ret"),
            (0x1000_0160, "push   ebp"),
            (0x1000_0161, "ret"),
        ]
    );
    assert_eq!(l.at[&0x1000_0141], 3);
    assert_eq!(l.at.len(), 6);
}

#[test]
fn a_section_is_swept_up_to_the_smaller_of_its_virtual_and_raw_size() {
    let bytes: &[(usize, &[u8])] = &[(0x100, &[0x55, 0x8b, 0xec, 0xc3, 0x90, 0x90])];
    // Virtual size smaller than the raw data: the tail is not swept.
    let img = flat(vec![section(".t", 0x100, 3, 0x100, 6, CODE_FLAGS)], bytes);
    let got = sta_mission_convert::disasm::Listing::from_image(&img).unwrap().ins;
    assert_eq!(
        got,
        [
            (0x1000_0100, "push   ebp".to_string()),
            (0x1000_0101, "mov    ebp,esp".to_string())
        ]
    );
    // Raw data smaller than the virtual size: the raw size ends the sweep.
    let img = flat(vec![section(".t", 0x100, 6, 0x100, 4, CODE_FLAGS)], bytes);
    let got = sta_mission_convert::disasm::Listing::from_image(&img).unwrap().ins;
    assert_eq!(got.len(), 3);
    // A virtual size of zero means the raw size.
    let img = flat(vec![section(".t", 0x100, 0, 0x100, 4, CODE_FLAGS)], bytes);
    assert_eq!(
        sta_mission_convert::disasm::Listing::from_image(&img)
            .unwrap()
            .ins
            .len(),
        3
    );
}

#[test]
fn a_pe_file_lists_through_the_image_trait() {
    use sta_mission_convert::fixture::{CODE, DATA, PeBuilder};
    let pe = Pe::from_bytes(
        PeBuilder::new(0x1000_0000)
            .section(".text", 0x1000, &[0x55, 0x8b, 0xec, 0xc3], CODE)
            .section(".rdata", 0x2000, &[0x55, 0xc3, 0x90, 0x90], DATA)
            .build()
            .bytes,
    )
    .expect("builder output parses");
    let l = Listing::from_image(&pe).unwrap();
    let got: Vec<(u32, &str)> = l.ins.iter().map(|(a, t)| (*a, t.as_str())).collect();
    assert_eq!(
        got,
        [
            (0x1000_1000, "push   ebp"),
            (0x1000_1001, "mov    ebp,esp"),
            (0x1000_1003, "ret"),
        ]
    );
}

#[test]
fn a_file_with_a_coff_symbol_table_is_refused() {
    use sta_mission_convert::fixture::{CODE, PeBuilder};
    use sta_mission_convert::{ErrorKind, Image};
    let bytes = PeBuilder::new(0x1000_0000)
        .section(".text", 0x1000, &[0x55, 0xc3], CODE)
        .build()
        .bytes;
    let pe_at = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    let with = |ptr: u32, n: u32| {
        let mut b = bytes.clone();
        b[pe_at + 12..pe_at + 16].copy_from_slice(&ptr.to_le_bytes());
        b[pe_at + 16..pe_at + 20].copy_from_slice(&n.to_le_bytes());
        Pe::from_bytes(b).expect("still parses")
    };
    // No table: listed. A count without a table pointer is no table either.
    for (ptr, n) in [(0, 0), (0, 5)] {
        let pe = with(ptr, n);
        assert!(!pe.has_symbol_table());
        assert_eq!(Listing::from_image(&pe).unwrap().ins.len(), 2);
    }
    // A table: refused with a decode error that names the reason.
    let pe = with(0x2000, 3);
    assert!(pe.has_symbol_table());
    let e = Listing::from_image(&pe).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Decode);
    assert!(e.message.contains("symbol table"), "{}", e.message);
    assert!(sta_mission_convert::disasm::disassemble(&pe).is_err());
}

#[test]
fn the_objdump_parser_keeps_the_lines_the_python_regex_keeps() {
    let text = "\nfile.dll:     file format pei-i386\n\n\nDisassembly of section .text:\n\n10001000 <.text>:\n\
10001000:\tpush   ebp\n10001001:\t  mov    ebp,esp  \n\t...\n 1000100a:\tret    \n10001010 <sym>:\nxyz:\tnot hex\n10001011:no tab\n1000g:\tbad\n";
    let l = Listing::parse_objdump(text);
    assert_eq!(
        l.ins,
        [
            (0x1000_1000, "push   ebp".to_string()),
            (0x1000_1001, "mov    ebp,esp".to_string()),
            (0x1000_100a, "ret".to_string()),
        ]
    );
    assert_eq!(l.at[&0x1000_100a], 2);
    // Pairs with the Python regex `^\s*([0-9a-f]+):\t(.*)$`: lower-case hex only, a tab right
    // after the colon, and a carriage return at the end is part of the text that is then stripped.
    let pairs: &[(&str, Option<(u32, &str)>)] = &[
        ("1000a:\tnop", Some((0x1000a, "nop"))),
        ("1000A:\tnop", None),
        ("  1000a:\tnop\r", Some((0x1000a, "nop"))),
        ("1000a: nop", None),
        ("1000a:\t", Some((0x1000a, ""))),
        ("x1000a:\tnop", None),
        ("0:\tadd    eax,0x1", Some((0, "add    eax,0x1"))),
    ];
    for (line, want) in pairs {
        let got = parse_objdump(line);
        let want: Vec<(u32, String)> = want.iter().map(|&(a, t)| (a, t.to_string())).collect();
        assert_eq!(got, want, "line {line:?}");
    }
    // Of two equal addresses the later wins, as in the Python dict.
    let l = Listing::new(vec![(1, "a".into()), (2, "b".into()), (1, "c".into())]);
    assert_eq!(l.at[&1], 2);
}
