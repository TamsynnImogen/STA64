//! `odf_fmt`: format ODF files (EDT-06). See `sta_tools::odf_fmt` for the options.

fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let code = sta_tools::odf_fmt::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
