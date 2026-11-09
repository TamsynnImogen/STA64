//! `sta-bench`: list, run, check, session, drive, diff (docs/TESTBENCH.md).

fn main() {
    std::process::exit(sta_bench::cli::main(std::env::args().skip(1).collect()));
}
