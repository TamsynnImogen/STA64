//! The engine's C-header config files (`RTS_CFG.h`, `ART_CFG.h`, `game_cfg.h`,
//! ...): `type NAME = value;` and `#define NAME value`, with `//` and `/* */`
//! comments. Values are kept as text; arrays and expressions are kept raw.

use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct CfgH {
    values: HashMap<String, String>,
}

impl CfgH {
    pub fn parse(text: &str) -> CfgH {
        let mut values = HashMap::new();
        let text = strip_block_comments(text);
        for line in text.lines() {
            let line = match line.find("//") {
                Some(i) => &line[..i],
                None => line,
            }
            .trim();
            if let Some(rest) = line.strip_prefix("#define") {
                let mut parts = rest.trim().splitn(2, char::is_whitespace);
                if let (Some(name), Some(value)) = (parts.next(), parts.next()) {
                    values.insert(name.to_owned(), value.trim().trim_matches('"').to_owned());
                }
                continue;
            }
            let Some((lhs, rhs)) = line.split_once('=') else { continue };
            let Some(name) = lhs.split_whitespace().last() else { continue };
            let name = name.split('[').next().unwrap_or(name);
            let value = rhs.trim().trim_end_matches(';').trim().trim_matches('"');
            values.insert(name.to_owned(), value.to_owned());
        }
        CfgH { values }
    }

    pub fn raw(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    pub fn f32(&self, name: &str) -> Option<f32> {
        self.raw(name)?.trim_end_matches(['f', 'F']).parse().ok()
    }

    pub fn i64(&self, name: &str) -> Option<i64> {
        self.raw(name)?.parse().ok()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

fn strip_block_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_values() {
        let c = CfgH::parse(
            "// CAMERA\nfloat MIN_TACT_ZOOM = 1.15;   // comment\nint SCROLL_BORDER_WIDTH = 8;\n\
             /* block\nint HIDDEN = 1; */\n#define STARFIELD_ODF_NAME \"stars_default.odf\"\n\
             float STARFIELD_GEOMETRY_SCALE = STAR_FAR_CLIP_PLANE*0.02;\nfloat X = 2.5f;\n",
        );
        assert_eq!(c.f32("MIN_TACT_ZOOM"), Some(1.15));
        assert_eq!(c.i64("SCROLL_BORDER_WIDTH"), Some(8));
        assert_eq!(c.raw("STARFIELD_ODF_NAME"), Some("stars_default.odf"));
        assert_eq!(c.raw("STARFIELD_GEOMETRY_SCALE"), Some("STAR_FAR_CLIP_PLANE*0.02"));
        assert_eq!(c.f32("X"), Some(2.5));
        assert!(c.raw("HIDDEN").is_none());
    }
}
