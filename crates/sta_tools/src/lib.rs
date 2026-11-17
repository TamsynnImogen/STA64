//! Modder tools (EDT lane): the command-line plumbing shared by the tools
//! ([`fsutil`]), the ODF formatter ([`odf_fmt`], bin `odf_fmt`), and with the
//! `shell` feature the tool shell of EDT-01 ([`shell`], bin `tool_shell`).

pub mod fsutil;
pub mod odf_fmt;

#[cfg(feature = "shell")]
pub mod shell;
