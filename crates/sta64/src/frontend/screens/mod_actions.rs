//! Explicit external actions from the mod selection screen.

use bevy::prelude::*;

use super::{Act, Browse, ScreenData};
use crate::ui::notices::{Notice, Severity};
use crate::ui::theme::ButtonClicked;

/// Platforms with a direct, argument-based URL opener.
pub(super) const WEBSITE_SUPPORTED: bool = cfg!(any(target_os = "linux", target_os = "macos"));

/// Only a web URL from a mod's metadata may be handed to the platform opener.
pub(super) fn website_url(website: &str) -> Option<&str> {
    let url = website.trim();
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains('\\')
        || url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }
    Some(url)
}

/// Actions only run in response to the theme's clicked-button message.
pub(super) fn on_external_actions(
    mut clicks: MessageReader<ButtonClicked>,
    acts: Query<&Act>,
    browse: Res<Browse>,
    data: Res<ScreenData>,
    flow: Option<Res<crate::game::flow::FlowConfig>>,
    mut notices: Option<ResMut<Messages<Notice>>>,
) {
    for ButtonClicked(entity) in clicks.read() {
        let Ok(act) = acts.get(*entity) else { continue };
        let mod_entry = browse
            .mod_row
            .checked_sub(1)
            .and_then(|index| data.mods.get(index));
        let outcome = match act {
            Act::VisitModWebsite => {
                let url = mod_entry
                    .and_then(|entry| entry.website.as_deref())
                    .and_then(website_url);
                match url {
                    Some(url) => open_website(url).map(|()| format!("Opening {url}")),
                    None => Err("This mod has no valid HTTP(S) website".to_owned()),
                }
            }
            Act::CreateModShortcut => {
                if let Some(reason) = mod_entry.and_then(|entry| entry.blocked.as_deref()) {
                    Err(format!("Shortcut unavailable: {reason}"))
                } else {
                    let folder = mod_entry.map(|entry| entry.folder.as_str());
                    let name = mod_entry.map_or("Base game", |entry| entry.name.as_str());
                    create_shortcut(
                        flow.as_ref()
                            .and_then(|config| config.install_root.as_deref()),
                        folder,
                        name,
                        browse.mod_no_intro,
                        browse.mod_no_ai,
                    )
                    .map(|path| format!("Shortcut created: {}", path.display()))
                }
            }
            _ => continue,
        };
        if let Some(notices) = &mut notices {
            let (severity, message) = match outcome {
                Ok(message) => (Severity::Info, message),
                Err(message) => (Severity::Bad, message),
            };
            notices.write(Notice::new(severity, message));
        }
    }
}

fn open_website(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err("Opening websites is unavailable on this platform".to_owned());

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let mut child = std::process::Command::new(opener)
            .arg(url)
            .spawn()
            .map_err(|error| format!("Could not open website: {error}"))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn desktop_value(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\\' => "\\\\".to_owned(),
            '\n' => "\\n".to_owned(),
            '\r' => "\\r".to_owned(),
            '\t' => "\\t".to_owned(),
            c if c.is_control() => " ".to_owned(),
            c => c.to_string(),
        })
        .collect()
}

/// Desktop Entry quoted arguments require escaping even though no shell is used.
#[cfg(target_os = "linux")]
fn desktop_arg(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err("Shortcut argument contains a control character".to_owned());
    }
    let mut result = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                result.push('\\');
                result.push(ch);
            }
            '%' => result.push_str("%%"),
            _ => result.push(ch),
        }
    }
    result.push('"');
    Ok(result)
}

#[cfg(target_os = "linux")]
fn shortcut_entry(
    executable: &std::path::Path,
    install_root: Option<&std::path::Path>,
    folder: Option<&str>,
    name: &str,
    no_intro: bool,
    no_ai: bool,
) -> Result<String, String> {
    let work_dir = executable
        .parent()
        .and_then(std::path::Path::to_str)
        .ok_or("Executable directory is not available")?;
    let executable = executable.to_str().ok_or("Executable path is not UTF-8")?;
    let mut args = vec![desktop_arg(executable)?];
    if let Some(root) = install_root {
        args.push("--install-root".to_owned());
        args.push(desktop_arg(
            root.to_str().ok_or("Install path is not UTF-8")?,
        )?);
    }
    if let Some(folder) = folder {
        if folder.starts_with("--")
            || folder.is_empty()
            || folder.contains('/')
            || folder.contains('\\')
        {
            return Err("Mod folder cannot be used as a shortcut argument".to_owned());
        }
        args.push("--mod".to_owned());
        args.push(desktop_arg(folder)?);
    }
    if no_intro {
        args.push("--nointro".to_owned());
    }
    if no_ai {
        args.push("--noai".to_owned());
    }
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={}\nPath={}\nTerminal=false\nCategories=Game;\n",
        desktop_value(&format!("STA64 - {name}")),
        desktop_value(&args.join(" ")),
        desktop_value(work_dir)
    ))
}

#[cfg(target_os = "linux")]
fn create_shortcut(
    install_root: Option<&std::path::Path>,
    folder: Option<&str>,
    name: &str,
    no_intro: bool,
    no_ai: bool,
) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    let executable = std::env::current_exe()
        .map_err(|error| format!("Cannot find STA64 executable: {error}"))?;
    let content = shortcut_entry(&executable, install_root, folder, name, no_intro, no_ai)?;
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share"))
        })
        .ok_or("Cannot locate the user application directory")?;
    let applications = data_home.join("applications");
    std::fs::create_dir_all(&applications)
        .map_err(|error| format!("Cannot create application directory: {error}"))?;
    let slug: String = folder
        .unwrap_or("base")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "mod" } else { slug };
    for number in 1..=999 {
        let suffix = if number == 1 {
            String::new()
        } else {
            format!("-{number}")
        };
        let path = applications.join(format!("sta64-{slug}{suffix}.desktop"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(content.as_bytes()) {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(format!("Cannot write {}: {error}", path.display()));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Cannot create {}: {error}", path.display())),
        }
    }
    Err("Too many shortcuts already exist for this mod".to_owned())
}

#[cfg(not(target_os = "linux"))]
fn create_shortcut(
    _install_root: Option<&std::path::Path>,
    _folder: Option<&str>,
    _name: &str,
    _no_intro: bool,
    _no_ai: bool,
) -> Result<std::path::PathBuf, String> {
    Err("Creating shortcuts is available on Linux only".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn website_rejects_non_web_schemes_and_bad_authorities() {
        assert_eq!(
            website_url(" https://example.org/a?b=1 "),
            Some("https://example.org/a?b=1")
        );
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https:///missing",
            "https://x y",
            "https://user@example.org",
            "https://example.org\\x",
        ] {
            assert_eq!(website_url(bad), None, "{bad}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_entry_quotes_metadata_and_launch_options() {
        use std::path::Path;
        let entry = shortcut_entry(
            Path::new("/opt/STA 64/$run"),
            Some(Path::new("/games/Armada %2")),
            Some("Fleet \"Ops\" %U"),
            "Fleet\nOps",
            true,
            true,
        )
        .unwrap();
        assert!(entry.contains("Name=STA64 - Fleet\\nOps\n"));
        let exec = r#""/opt/STA 64/\$run" --install-root "/games/Armada %%2" --mod "Fleet \"Ops\" %%U" --nointro --noai"#;
        assert!(entry.contains(&format!("Exec={}\n", desktop_value(exec))));
        assert!(entry.contains("Path=/opt/STA 64\n"));
        assert!(
            shortcut_entry(
                Path::new("/bin/sta64"),
                None,
                Some("--headless"),
                "bad",
                false,
                false
            )
            .is_err()
        );
    }
}
