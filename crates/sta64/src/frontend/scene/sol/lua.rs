//! Load a menu scene's declarative Lua table once, without exposing mission APIs.

use mlua::{Table, Value, chunk::ChunkMode};

const SOURCE_MAX: usize = 64 * 1024;
const MEMORY_MAX: usize = 8 * 1024 * 1024;
const LIST_MAX: usize = 128;
const NAME_MAX: usize = 128;

#[derive(Debug, Default)]
pub(super) struct Script {
    pub(super) views: Option<Vec<View>>,
    pub(super) ships: Vec<Ship>,
    pub(super) sky: Option<String>,
    pub(super) stars: Option<u32>,
    pub(super) blend: Option<f32>,
}

#[derive(Debug)]
pub(super) struct View {
    pub(super) name: String,
    pub(super) camera: Vec<String>,
    pub(super) target: Option<Vec<String>>,
    pub(super) seconds: Option<f32>,
    pub(super) looped: bool,
    pub(super) fov: Option<f32>,
}

#[derive(Debug)]
pub(super) struct Ship {
    pub(super) name: String,
    pub(super) path: Vec<String>,
    pub(super) seconds: Option<f32>,
    pub(super) looped: bool,
}

pub(super) fn parse(source: &[u8]) -> Result<Script, String> {
    if source.len() > SOURCE_MAX {
        return Err(format!("menu script exceeds {SOURCE_MAX} bytes"));
    }
    let lua = sta_mission::sandbox::new_lua();
    lua.set_memory_limit(MEMORY_MAX)
        .map_err(|e| e.to_string())?;
    sta_mission::math::install(&lua, 0).map_err(|e| e.to_string())?;
    let root: Table = lua
        .load(source)
        .set_mode(ChunkMode::Text)
        .set_name("=menu scene")
        .eval()
        .map_err(|e| e.to_string())?;
    Ok(Script {
        views: optional_list(&root, "views", view)?,
        ships: optional_list(&root, "ships", ship)?.unwrap_or_default(),
        sky: optional_name(&root, "sky")?,
        stars: optional_stars(&root)?,
        blend: optional_number(&root, "blend", 0.0, 60.0)?,
    })
}

fn field(table: &Table, key: &str) -> Result<Value, String> {
    table.raw_get(key).map_err(|e| format!("{key}: {e}"))
}

fn name(value: Value, what: &str) -> Result<String, String> {
    let Value::String(value) = value else {
        return Err(format!("{what} must be a string"));
    };
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > NAME_MAX
        || !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'.' | b'_' | b'-'))
    {
        return Err(format!(
            "{what} must contain 1..={NAME_MAX} ASCII letters, digits, spaces, dots, underscores or hyphens"
        ));
    }
    String::from_utf8(bytes.to_vec()).map_err(|e| format!("{what}: {e}"))
}

fn required_name(table: &Table, key: &str) -> Result<String, String> {
    name(field(table, key)?, key)
}

fn optional_name(table: &Table, key: &str) -> Result<Option<String>, String> {
    match field(table, key)? {
        Value::Nil => Ok(None),
        value => name(value, key).map(Some),
    }
}

fn list<T>(
    table: Table,
    what: &str,
    mut item: impl FnMut(Value, &str) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    let len = table.raw_len();
    if len > LIST_MAX {
        return Err(format!("{what} must contain at most {LIST_MAX} entries"));
    }
    // `raw_len` alone can hide sparse entries; reject every non-array key too.
    let mut count = 0;
    for entry in table.clone().pairs::<Value, Value>() {
        let (key, _) = entry.map_err(|e| format!("{what}: {e}"))?;
        count += 1;
        if count > LIST_MAX
            || !matches!(key, Value::Integer(i) if i >= 1 && usize::try_from(i).ok().is_some_and(|i| i <= len))
        {
            return Err(format!(
                "{what} must be a dense array of at most {LIST_MAX} entries"
            ));
        }
    }
    if count != len {
        return Err(format!("{what} must be a dense array"));
    }
    (1..=len)
        .map(|i| {
            let label = format!("{what}[{i}]");
            let value = table.raw_get(i).map_err(|e| format!("{label}: {e}"))?;
            item(value, &label)
        })
        .collect()
}

fn optional_list<T>(
    table: &Table,
    key: &str,
    item: impl FnMut(Value, &str) -> Result<T, String>,
) -> Result<Option<Vec<T>>, String> {
    match field(table, key)? {
        Value::Nil => Ok(None),
        Value::Table(rows) => list(rows, key, item).map(Some),
        _ => Err(format!("{key} must be a table")),
    }
}

fn points(table: &Table, key: &str) -> Result<Vec<String>, String> {
    let Value::Table(rows) = field(table, key)? else {
        return Err(format!("{key} must be a table"));
    };
    let values = list(rows, key, name)?;
    if values.is_empty() {
        return Err(format!("{key} must contain at least one point"));
    }
    Ok(values)
}

fn optional_points(table: &Table, key: &str) -> Result<Option<Vec<String>>, String> {
    match field(table, key)? {
        Value::Nil => Ok(None),
        Value::Table(rows) => {
            let values = list(rows, key, name)?;
            if values.is_empty() {
                return Err(format!("{key} must contain at least one point"));
            }
            Ok(Some(values))
        }
        _ => Err(format!("{key} must be a table")),
    }
}

fn optional_number(table: &Table, key: &str, min: f32, max: f32) -> Result<Option<f32>, String> {
    let number = match field(table, key)? {
        Value::Nil => return Ok(None),
        Value::Integer(n) => n as f32,
        Value::Number(n) => n as f32,
        _ => return Err(format!("{key} must be a number")),
    };
    if !(min..=max).contains(&number) {
        return Err(format!("{key} must be finite and within {min}..={max}"));
    }
    Ok(Some(number))
}

fn optional_stars(table: &Table) -> Result<Option<u32>, String> {
    let number = match field(table, "stars")? {
        Value::Nil => return Ok(None),
        Value::Integer(n) => n as f64,
        Value::Number(n) => n,
        _ => return Err("stars must be an integer".into()),
    };
    if !number.is_finite() || number.fract() != 0.0 || !(0.0..=10_000.0).contains(&number) {
        return Err("stars must be an integer within 0..=10000".into());
    }
    Ok(Some(number as u32))
}

fn looped(table: &Table) -> Result<bool, String> {
    match field(table, "loop")? {
        Value::Nil => Ok(false),
        Value::Boolean(value) => Ok(value),
        _ => Err("loop must be a boolean".into()),
    }
}

fn row(value: Value, what: &str) -> Result<Table, String> {
    match value {
        Value::Table(row) => Ok(row),
        _ => Err(format!("{what} must be a table")),
    }
}

fn view(value: Value, what: &str) -> Result<View, String> {
    let row = row(value, what)?;
    Ok(View {
        name: required_name(&row, "name")?,
        camera: points(&row, "camera")?,
        target: optional_points(&row, "target")?,
        seconds: optional_number(&row, "seconds", 0.001, f32::MAX)?,
        looped: looped(&row)?,
        fov: optional_number(&row, "fov", 1.0, 150.0)?,
    })
}

fn ship(value: Value, what: &str) -> Result<Ship, String> {
    let row = row(value, what)?;
    Ok(Ship {
        name: required_name(&row, "name")?,
        path: points(&row, "path")?,
        seconds: optional_number(&row, "seconds", 0.001, f32::MAX)?,
        looped: looped(&row)?,
    })
}
