//! Advisory same-file ODF duplicate report for loose UTF-8 inputs.

use std::fs;
use std::io::{self, Read};

const USAGE: &str = "Usage: sta-odf-duplicates [--deny-duplicates] [--] FILE...\n       sta-odf-duplicates --help\nUse - for stdin. Repeated commands are legal; later values win.";

fn run() -> i32 {
    let mut deny = false;
    let mut end_options = false;
    let mut paths = Vec::new();
    for argument in std::env::args_os().skip(1) {
        let Ok(argument) = argument.into_string() else {
            eprintln!("sta-odf-duplicates: non-UTF-8 argument\n{USAGE}");
            return 2;
        };
        if !end_options {
            match argument.as_str() {
                "--help" => {
                    println!("{USAGE}");
                    return 0;
                }
                "--deny-duplicates" => {
                    deny = true;
                    continue;
                }
                "--" => {
                    end_options = true;
                    continue;
                }
                "-" => {}
                _ if argument.starts_with('-') => {
                    eprintln!("sta-odf-duplicates: unknown option {argument}\n{USAGE}");
                    return 2;
                }
                _ => {}
            }
        }
        paths.push(argument);
    }
    if paths.is_empty() {
        eprintln!("sta-odf-duplicates: expected at least one file\n{USAGE}");
        return 2;
    }

    // Read all inputs before reporting, so a later read/encoding error takes
    // precedence over duplicate findings in an earlier file.
    let mut documents = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = if path == "-" {
            let mut bytes = Vec::new();
            if let Err(error) = io::stdin().read_to_end(&mut bytes) {
                eprintln!("sta-odf-duplicates: stdin: {error}");
                return 2;
            }
            bytes
        } else {
            match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    eprintln!("sta-odf-duplicates: {path}: {error}");
                    return 2;
                }
            }
        };
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                eprintln!("sta-odf-duplicates: {path}: input is not UTF-8");
                return 2;
            }
        };
        documents.push((path, sta_odf::parse(&text)));
    }

    let mut found = false;
    for (path, document) in &documents {
        for duplicate in document.duplicates() {
            found = true;
            println!(
                "{path}:{}: {} overrides {}:{}",
                duplicate.replacement.line,
                duplicate.replacement.key,
                duplicate.previous.line,
                duplicate.previous.key
            );
        }
    }
    i32::from(deny && found)
}

fn main() {
    std::process::exit(run());
}
