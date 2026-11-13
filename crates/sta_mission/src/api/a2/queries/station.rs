//! Bounded native-family classification; independent of simulation actor kind.
//! The owning API README records the static evidence and bridge limitations.

use mlua::{Function, Lua, MultiValue};

use crate::api::Shared;
use crate::api::a2::objects::handle;

pub(in crate::api::a2) fn craft_family(class: &str) -> Option<bool> {
    match class.to_ascii_lowercase().as_str() {
        "cargo_ship" | "colony" | "constructionrig" | "craft" | "evolver" | "freighter"
        | "hover" | "minearray" | "mining" | "p_mining" | "planet" | "pod" | "producer"
        | "repairship" | "research" | "scavenger" | "sensor" | "shipyard" | "starbase"
        | "trading_station" | "wingman" => Some(true),
        "areaeffect_obj" | "asteroidfield" | "background_obj" | "blackhole_obj"
        | "constplaceholder" | "i76building" | "ionstorm_obj" | "latinum" | "nebula_obj"
        | "planet_obj" | "planet_shield" | "scrap" | "u_ball_obj" | "upgrade" | "wormhole_obj" => {
            Some(false)
        }
        _ => None,
    }
}

pub(super) fn function(lua: &Lua, ctx: &Shared) -> mlua::Result<Function> {
    let c = ctx.clone();
    lua.create_function(move |_, args: MultiValue| {
        let id = handle("IsStation", &args, 1)?;
        let mut c = c.borrow_mut();
        let Some(unit) = c.alive(id) else {
            return Ok(false);
        };
        // Core represents a construction proxy with the final definition.
        // Native ConstructionObject does not have the Craft type guard.
        if unit.progress.is_some() {
            return Ok(false);
        }
        let def = c.def(unit).ok_or_else(|| {
            mlua::Error::runtime(format!(
                "A2.IsStation: object {} has no unit definition",
                id.0
            ))
        })?;
        if !def.is_starbase {
            return Ok(false);
        }
        match craft_family(&def.class.0) {
            Some(value) => Ok(value),
            None => {
                let reason = format!("A2.IsStation: unclassified native class {}", def.class.0);
                c.unsupported(&reason);
                Err(mlua::Error::runtime(reason))
            }
        }
    })
}

#[cfg(test)]
mod tests;
