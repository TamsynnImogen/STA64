//! The Armada I part of rendering (`render_expr` hook, `render_failed` of
//! `tools/a1_to_lua.py`).
//!
//! The Python replaces `dsl.render_expr` by a wrapper that handles the Armada I node kinds and
//! delegates everything else to the Armada II renderer, whose recursion goes through the patched
//! name. LUA-15h's renderer must call [`intercept`] at the top of its `render_expr` (and so of
//! every recursion, `render_lv` and `render_stmt` included) when it renders for Armada I; that is
//! the same as the patch.

use crate::format::{lua_str, lua_str_py};
use crate::pyval::Py;
use crate::{DecodeError, Result};

/// The wrapper's own cases: `Some(result)` when `a1_to_lua.render_expr` answers for `e` itself
/// (`nil`, `undecodedcall`, the kinds that cannot be rendered), `None` when the Armada II renderer
/// answers. `ctx` is `"num"`, `"bool"` or `"arg"`.
pub fn intercept(e: &Py, ctx: &str) -> Option<Result<String>> {
    let k = match e.item(0) {
        Ok(k) => k,
        Err(err) => return Some(Err(err)),
    };
    match k.as_str() {
        Some("nil") => Some(Ok((if ctx == "bool" {
            "false"
        } else if ctx == "num" {
            "0"
        } else {
            "nil"
        })
        .to_owned())),
        Some("undecodedcall") => Some(undecoded(e, ctx)),
        Some(kind @ ("saved" | "worldoff" | "impaddr" | "rule" | "index" | "impslot" | "mem" | "stackslot")) => {
            Some(Err(DecodeError::new(format!("cannot render {kind}"))))
        }
        _ => None,
    }
}

fn undecoded(e: &Py, ctx: &str) -> Result<String> {
    let s = format!("UD({}, {})", lua_str_py(e.item(1)?)?, lua_str_py(e.item(2)?)?);
    Ok(if ctx == "bool" {
        format!("T({s})")
    } else if ctx == "num" {
        format!("N({s})")
    } else {
        s
    })
}

/// `render_failed(name, why, calls)`: the stub of a function that is not decoded: it counts
/// itself and the calls it would have made when it runs.
pub fn render_failed(name: &str, why: &str, calls: &[String]) -> Vec<String> {
    let lst = calls.iter().map(|c| lua_str(c)).collect::<Vec<_>>().join(", ");
    vec![
        format!("function M.{name}(...)"),
        format!("    M.notDecoded({}, {}, {{ {lst} }})", lua_str(name), lua_str(why)),
        "    return 0".to_owned(),
        "end".to_owned(),
    ]
}
