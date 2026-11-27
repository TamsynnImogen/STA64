//! `sta_mission_convert --install <game root> [--out <folder>] [--only <mission>]...`: converts the
//! player's own Armada I or II mission DLLs to Lua, with no Python and no `objdump` (see
//! [`sta_mission_convert::cli`] for the arguments, the output and the exit codes).

use std::io::Write;
use std::path::Path;

use sta_mission_convert::{cli, disasm, Pe};

fn main() {
    // paths stay as the operating system has them: a lossy conversion could write somewhere else
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mut listing = |p: &Path| disasm::disassemble(&Pe::open(p)?);
    let (stdout, stderr) = (std::io::stdout(), std::io::stderr());
    let (mut out, mut err) = (stdout.lock(), stderr.lock());
    let code = cli::run(
        &args,
        &cli::Env::from_process(),
        &mut cli::Io { out: &mut out, err: &mut err },
        &mut listing,
    );
    let _ = out.flush();
    let _ = err.flush();
    std::process::exit(code);
}
