//! Path handling for a tool that writes next to a player's games (LUA-15f).
//!
//! Safety here is by construction: every folder the tool writes into, reads roots from or
//! compares is first turned into one canonical form by [`resolve`], and containment is decided on
//! those forms only.
//!
//! [`resolve`] makes a path absolute, removes `.` and `..` **lexically after** the longest
//! existing prefix has been resolved through the file system (symbolic links followed, the real
//! spelling and case of what exists taken), so that `<existing>/missing/../<game>/x` is
//! `<existing>/<game>/x` and is seen to lie inside the game. A path that cannot be resolved (a
//! link that leads nowhere, a loop) is an error, not a guess. On Windows the verbatim `\\?\`
//! prefix the file system adds is removed where the plain form means the same.

use std::path::{Component, Path, PathBuf};

/// `\\?\C:\x` as `C:\x`, `\\?\UNC\srv\share\x` as `\\srv\share\x`; anything else unchanged.
pub fn simplify_verbatim(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        let b = rest.as_bytes();
        if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'\\') {
            return rest.to_owned();
        }
    }
    s.to_owned()
}

fn simplified(p: PathBuf) -> PathBuf {
    if cfg!(windows)
        && let Some(s) = p.to_str()
    {
        return PathBuf::from(simplify_verbatim(s));
    }
    p
}

/// The canonical form of `p` (see the module documentation).
pub fn resolve(p: &Path) -> Result<PathBuf, String> {
    let abs = std::path::absolute(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut cur = PathBuf::new();
    // the part below `cur` that does not exist (so cannot be a link): only names, no `..`
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    for c in abs.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => cur.push(c.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if tail.pop().is_none() {
                    cur.pop();
                }
            }
            Component::Normal(n) => {
                if !tail.is_empty() {
                    tail.push(n.to_owned());
                    continue;
                }
                let cand = cur.join(n);
                if std::fs::symlink_metadata(&cand).is_err() {
                    tail.push(n.to_owned());
                } else {
                    let real = std::fs::canonicalize(&cand)
                        .map_err(|e| format!("{}: cannot be resolved ({e})", cand.display()))?;
                    cur = simplified(real);
                }
            }
        }
    }
    cur.extend(tail);
    Ok(cur)
}

/// Whether `path` is `root` or below it: component-wise on resolved paths, ignoring case where
/// the platform's file systems usually do (Windows, macOS).
pub fn contains(root: &Path, path: &Path) -> bool {
    let ci = cfg!(any(windows, target_os = "macos"));
    let mut r = root.components();
    let mut p = path.components();
    loop {
        match (r.next(), p.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(a), Some(b)) => {
                let same = if ci {
                    a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
                } else {
                    a == b
                };
                if !same {
                    return false;
                }
            }
        }
    }
}

/// Whether the folder holds a game's executable (`Armada*.exe`) at its top or in `Data/`: a game
/// root, or the engine folder of a Fleet Operations style one.
pub fn looks_like_game_root(dir: &Path) -> bool {
    fn has_exe(d: &Path) -> bool {
        std::fs::read_dir(d).is_ok_and(|rd| {
            rd.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                n.starts_with("armada") && n.ends_with(".exe")
            })
        })
    }
    if has_exe(dir) {
        return true;
    }
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("data") && has_exe(&e.path()))
    })
}

/// The game root `path` is at or inside, if any: the nearest folder on the way up that
/// [`looks_like_game_root`].
pub fn game_root_above(path: &Path) -> Option<PathBuf> {
    path.ancestors().filter(|a| a.is_dir()).find(|a| looks_like_game_root(a)).map(Path::to_path_buf)
}
