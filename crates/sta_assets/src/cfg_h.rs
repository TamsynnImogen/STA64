//! C-header config values, retained as source text for scalar and array readers.
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct CfgH {
    values: HashMap<String, String>,
}

impl CfgH {
    pub fn parse(text: &str) -> CfgH {
        let text = strip_comments(text);
        let mut values = HashMap::new();
        let mut stmt = String::new();
        let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
        for line in text.lines() {
            let line = line.trim();
            if !quoted && let Some(directive) = line.strip_prefix('#') {
                let directive = directive.trim_start();
                if let Some(rest) = directive
                    .strip_prefix("define")
                    .filter(|rest| rest.starts_with(char::is_whitespace))
                {
                    let mut parts = rest.trim().splitn(2, char::is_whitespace);
                    if let (Some(name), Some(value)) = (parts.next(), parts.next()) {
                        values.insert(name.to_owned(), scalar(value.trim()).to_owned());
                    }
                }
                // Includes are expanded by the owning VFS loader. This is not
                // a C preprocessor; guards/pragmas must not pollute a declaration.
                continue;
            }
            for ch in line.chars().chain(std::iter::once('\n')) {
                if ch == '"' && !escaped {
                    quoted = !quoted;
                }
                if !quoted {
                    match ch {
                        '{' => depth += 1,
                        '}' => depth = depth.saturating_sub(1),
                        ';' if depth == 0 => {
                            insert(&mut values, &stmt);
                            stmt.clear();
                            escaped = false;
                            continue;
                        }
                        _ => {}
                    }
                }
                escaped = quoted && ch == '\\' && !escaped;
                stmt.push(ch);
            }
            if depth == 0
                && !quoted
                && let Some((_, rhs)) = stmt.split_once('=')
                && !rhs.trim().is_empty()
            {
                insert(&mut values, &stmt);
                stmt.clear();
            }
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
        self.eval_i64(name)
    }
    pub fn eval_i64(&self, name: &str) -> Option<i64> {
        self.eval_named(name, &mut Vec::new())
    }
    fn eval_named(&self, name: &str, stack: &mut Vec<String>) -> Option<i64> {
        if stack.len() >= 128 || stack.iter().any(|item| item == name) {
            return None;
        }
        stack.push(name.to_owned());
        let result = self.raw(name).and_then(|raw| Expr::new(raw, self, stack).parse());
        stack.pop();
        result
    }
    /// Strictly reads alternating `"key", integer_expression` entries in source order.
    pub fn integer_pairs(&self, name: &str) -> Option<Vec<(String, i64)>> {
        let raw = self.raw(name)?.trim();
        let body = raw.strip_prefix('{')?.strip_suffix('}')?;
        let fields = split_top(body)?;
        if fields.len() % 2 != 0 {
            return None;
        }
        let mut rows = Vec::new();
        for pair in fields.chunks_exact(2) {
            let key = string_literal(pair[0].trim())?;
            let val = Expr::new(pair[1].trim(), self, &mut Vec::new()).parse()?;
            rows.push((key, val));
        }
        Some(rows)
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}
fn scalar(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(value)
}
fn insert(values: &mut HashMap<String, String>, statement: &str) {
    let Some((lhs, rhs)) = statement.split_once('=') else {
        return;
    };
    let Some(name) = lhs.split_whitespace().last() else {
        return;
    };
    let name = name.split('[').next().unwrap_or(name);
    if !name.is_empty() {
        values.insert(name.to_owned(), scalar(rhs.trim()).to_owned());
    }
}
fn strip_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let (mut quoted, mut escaped) = (false, false);
    while let Some(ch) = chars.next() {
        if ch == '"' && !escaped {
            quoted = !quoted;
        }
        if !quoted && ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
            escaped = false;
            continue;
        }
        if !quoted && ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = '\0';
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                }
                if prev == '*' && c == '/' {
                    break;
                }
                prev = c;
            }
            out.push(' ');
            escaped = false;
            continue;
        }
        escaped = quoted && ch == '\\' && !escaped;
        out.push(ch);
    }
    out
}
fn split_top(input: &str) -> Option<Vec<&str>> {
    let (mut braces, mut parens, mut quoted, mut escaped) = (0usize, 0usize, false, false);
    let (mut start, mut parts) = (0usize, Vec::new());
    for (i, ch) in input.char_indices() {
        if ch == '"' && !escaped {
            quoted = !quoted;
        }
        if !quoted {
            match ch {
                '{' => braces += 1,
                '}' => braces = braces.checked_sub(1)?,
                '(' => parens += 1,
                ')' => parens = parens.checked_sub(1)?,
                ',' if braces == 0 && parens == 0 => {
                    parts.push(&input[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        escaped = quoted && ch == '\\' && !escaped;
    }
    if quoted || braces != 0 || parens != 0 {
        return None;
    }
    if !input[start..].trim().is_empty() {
        parts.push(&input[start..]);
    }
    Some(parts)
}
fn string_literal(input: &str) -> Option<String> {
    let inner = input.strip_prefix('"')?.strip_suffix('"')?;
    let mut chars = inner.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        out.push(match c {
            '\\' => match chars.next()? {
                '"' => '"',
                '\\' => '\\',
                _ => return None,
            },
            '"' => return None,
            other => other,
        });
    }
    Some(out)
}
struct Expr<'a, 's> {
    bytes: &'a [u8],
    pos: usize,
    cfg: &'a CfgH,
    stack: &'s mut Vec<String>,
}
impl<'a, 's> Expr<'a, 's> {
    fn new(input: &'a str, cfg: &'a CfgH, stack: &'s mut Vec<String>) -> Self {
        Self {
            bytes: input.as_bytes(),
            pos: 0,
            cfg,
            stack,
        }
    }
    fn parse(mut self) -> Option<i64> {
        let value = self.add()?;
        self.ws();
        (self.pos == self.bytes.len()).then_some(value)
    }
    fn ws(&mut self) {
        while self.bytes.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }
    fn eat(&mut self, b: u8) -> bool {
        self.ws();
        if self.bytes.get(self.pos) == Some(&b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn add(&mut self) -> Option<i64> {
        let mut v = self.mul()?;
        loop {
            v = if self.eat(b'+') {
                v.checked_add(self.mul()?)?
            } else if self.eat(b'-') {
                v.checked_sub(self.mul()?)?
            } else {
                return Some(v);
            };
        }
    }
    fn mul(&mut self) -> Option<i64> {
        let mut v = self.unary()?;
        loop {
            v = if self.eat(b'*') {
                v.checked_mul(self.unary()?)?
            } else if self.eat(b'/') {
                v.checked_div(self.unary()?)?
            } else {
                return Some(v);
            };
        }
    }
    fn unary(&mut self) -> Option<i64> {
        // One shared budget covers parentheses, signs and referenced macros;
        // separate per-expression budgets could multiply into an unsafe stack.
        if self.stack.len() >= 128 {
            return None;
        }
        self.stack.push(String::new());
        let result = self.atom();
        self.stack.pop();
        result
    }
    fn atom(&mut self) -> Option<i64> {
        if self.eat(b'+') {
            return self.unary();
        }
        if self.eat(b'-') {
            return self.unary()?.checked_neg();
        }
        if self.eat(b'(') {
            let v = self.add()?;
            return self.eat(b')').then_some(v);
        }
        self.ws();
        let start = self.pos;
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
        {
            self.pos += 1;
        }
        let token = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?;
        if token.is_empty() {
            return None;
        }
        if token.bytes().all(|c| c.is_ascii_digit()) {
            token.parse().ok()
        } else if token.as_bytes()[0].is_ascii_alphabetic() || token.starts_with('_') {
            self.cfg.eval_named(token, self.stack)
        } else {
            None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_compatibility() {
        let c = CfgH::parse(
            "float Z = 1.15; // x\n int W = 8;\n/* int H = 3; */\n#define SOUND \"stars.odf\"\nfloat X = 2.5f;",
        );
        assert_eq!(c.f32("Z"), Some(1.15));
        assert_eq!(c.i64("W"), Some(8));
        assert_eq!(c.raw("SOUND"), Some("stars.odf"));
        assert_eq!(c.f32("X"), Some(2.5));
        assert_eq!(c.raw("H"), None);
    }
    #[test]
    fn officer_rows_and_expressions() {
        let c = CfgH::parse(
            r#"#define SINGLE_PLAYER_OFFICER_LIMIT 600
            int MAX_OFFICERS = 700;
            Officer MP_OFFICER_LIMIT[4] = {
              "ol_custom", 0, // zero
              "ol_restricted", SINGLE_PLAYER_OFFICER_LIMIT / 2,
              /* row */ "ol_normal", SINGLE_PLAYER_OFFICER_LIMIT,
              "ol_max", MAX_OFFICERS,
            };
            int MAX_OFFICERS = 900;"#,
        );
        assert!(c.raw("MP_OFFICER_LIMIT").unwrap().starts_with('{'));
        assert_eq!(c.i64("MAX_OFFICERS"), Some(900));
        assert_eq!(
            c.integer_pairs("MP_OFFICER_LIMIT"),
            Some(vec![
                ("ol_custom".into(), 0),
                ("ol_restricted".into(), 300),
                ("ol_normal".into(), 600),
                ("ol_max".into(), 900)
            ])
        );
    }
    #[test]
    fn invalid_data_fails_closed() {
        let c = CfgH::parse(
            r#"int A = B+1; int B = A-1;
            int DIV = 1/0; int OVER = 9223372036854775807+1;
            int GOOD = -(2+3)*+4;
            Thing BAD[2] = {"first",GOOD,"second",UNKNOWN};
            Thing MALFORMED[1] = {"first",1,2};
            #define URL "http://example/*literal*/""#,
        );
        for key in ["A", "B", "DIV", "OVER"] {
            assert_eq!(c.i64(key), None);
        }
        assert_eq!(c.i64("GOOD"), Some(-20));
        assert_eq!(c.integer_pairs("BAD"), None);
        assert_eq!(c.integer_pairs("MALFORMED"), None);
        assert_eq!(c.raw("URL"), Some("http://example/*literal*/"));
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::CfgH;
    #[test]
    fn guards_pragmas_and_tab_defines_do_not_poison_values() {
        let cfg = CfgH::parse(
            "#ifndef CONFIG_H\n#define CONFIG_H\n#pragma once\n#define\tBASE 600\nint LIMIT = BASE / 2;\n#endif\nint RAW = 8\n",
        );
        assert_eq!(cfg.i64("LIMIT"), Some(300));
        assert_eq!(cfg.i64("RAW"), Some(8));
    }
    #[test]
    fn excessively_deep_expressions_fail_without_stack_overflow() {
        let text = format!("int VALUE = {}1{};", "(".repeat(2000), ")".repeat(2000));
        assert_eq!(CfgH::parse(&text).i64("VALUE"), None);
    }
    #[test]
    fn nested_macro_expressions_share_one_depth_budget() {
        let mut text = String::new();
        for index in 0..64 {
            text.push_str(&format!(
                "int V{index} = {}V{}{};\n",
                "(".repeat(64),
                index + 1,
                ")".repeat(64)
            ));
        }
        text.push_str("int V64 = 1;");
        assert_eq!(CfgH::parse(&text).i64("V0"), None);
    }
}
