//! Optional diagnostics over assignments written in one ODF file.

use std::collections::HashMap;

use crate::{Entry, RawOdf};

/// A same-file assignment and the immediately preceding assignment it replaces.
/// Both entries borrow their original spelling, line and value from [`RawOdf`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuplicateEntry<'a> {
    pub previous: &'a Entry,
    pub replacement: &'a Entry,
}

impl RawOdf {
    /// Replacements in source order, using the same ASCII case folding as ODF lookup.
    /// Includes are deliberately outside this same-file report.
    pub fn duplicates(&self) -> Vec<DuplicateEntry<'_>> {
        let mut latest: HashMap<String, usize> = HashMap::new();
        let mut duplicates = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some(previous) = latest.insert(entry.key.to_ascii_lowercase(), index) {
                duplicates.push(DuplicateEntry {
                    previous: &self.entries[previous],
                    replacement: entry,
                });
            }
        }
        duplicates
    }
}

#[cfg(test)]
mod tests {
    use crate::{parse, resolve};

    #[test]
    fn case_folded_chains_keep_original_lines_values_and_borrowed_entries() {
        let raw = parse(
            "// comment\nSpeed = 1\nother = 7\nspeed = 0 // explicit zero\nSPEED = \"a // b\"\n",
        );
        let duplicates = raw.duplicates();
        assert_eq!(duplicates.len(), 2);
        assert_eq!(
            (
                duplicates[0].previous.key.as_str(),
                duplicates[0].previous.line,
                duplicates[0].previous.value.as_str()
            ),
            ("Speed", 2, "1")
        );
        assert_eq!(
            (
                duplicates[0].replacement.key.as_str(),
                duplicates[0].replacement.line,
                duplicates[0].replacement.value.as_str()
            ),
            ("speed", 4, "0")
        );
        assert_eq!(
            (
                duplicates[1].previous.key.as_str(),
                duplicates[1].replacement.key.as_str()
            ),
            ("speed", "SPEED")
        );
        assert_eq!(duplicates[1].replacement.value, "\"a // b\"");
        assert!(std::ptr::eq(duplicates[1].previous, &raw.entries[2]));
        assert!(std::ptr::eq(duplicates[1].replacement, &raw.entries[3]));
    }

    #[test]
    fn include_override_is_legal_and_not_a_same_file_duplicate() {
        let child = "#include \"base.odf\"\nSPEED = 0\n";
        assert!(parse(child).duplicates().is_empty());
        let mut load = |name: &str| match name.to_ascii_lowercase().as_str() {
            "base.odf" => Some("speed = 1\n".to_owned()),
            "child.odf" => Some(child.to_owned()),
            _ => None,
        };
        assert_eq!(resolve("child", &mut load).unwrap().raw("speed"), Some("0"));
    }

    #[test]
    fn ascii_case_folding_does_not_merge_distinct_non_ascii_keys() {
        let raw = parse("Ä = 1\nä = 2\n");
        assert!(raw.duplicates().is_empty());
    }
}
