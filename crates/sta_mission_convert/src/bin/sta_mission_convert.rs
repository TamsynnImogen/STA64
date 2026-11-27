//! `sta_mission_convert --install <game root> [--out <folder>] [--only <mission>]...`: converts the
//! player's own Armada I or II mission DLLs to Lua, with no Python and no `objdump` (see
//! [`sta_mission_convert::cli`] for the arguments, the output and the exit codes).

use std::io::Write;

use sta_mission_convert::{cli, disasm};

fn main() {
    // paths stay as the operating system has them: a lossy conversion could write somewhere else
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    // the converter hands the decoder the file it has read: no DLL is read again for its listing
    let mut decoder = |_: &std::path::Path, pe: &sta_mission_convert::Pe| disasm::disassemble(pe);
    let (stdout, stderr) = (std::io::stdout(), std::io::stderr());
    let (mut out, mut err) = (stdout.lock(), stderr.lock());
    let code = cli::run_with(
        &args,
        &cli::Env::from_process(),
        &mut cli::Io { out: &mut out, err: &mut err },
        &mut decoder,
    );
    let _ = out.flush();
    let _ = err.flush();
    std::process::exit(code);
}
