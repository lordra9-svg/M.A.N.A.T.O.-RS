use std::path::Path;

/// One `key = value` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub value: String,
    pub line: usize,
}

impl Entry {
    pub fn key_lower(&self) -> String {
        self.key.trim().to_ascii_lowercase()
    }

    pub fn as_i32(&self) -> Option<i32> {
        self.value.trim().parse().ok()
    }

    pub fn as_f32(&self) -> Option<f32> {
        self.value.trim().parse().ok()
    }
}

/// A bracketed section such as `[Statedef 200]` or `[State 200, walk]`.
#[derive(Debug, Clone, Default)]
pub struct Section {
    /// Text inside the brackets exactly as written.
    pub header: String,
    pub line: usize,
    pub entries: Vec<Entry>,
}

impl Section {
    pub fn header_lower(&self) -> String {
        self.header.trim().to_ascii_lowercase()
    }

    /// First whitespace-delimited token of the header, lowercased.
    pub fn kind(&self) -> String {
        self.header_lower()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.key.trim().eq_ignore_ascii_case(key))
            .map(|e| e.value.as_str())
    }

    pub fn get_all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries
            .iter()
            .filter(move |e| e.key.trim().eq_ignore_ascii_case(key))
            .map(|e| e.value.as_str())
    }
}

/// A parsed INI-like text file. Preserves duplicate sections and entry order,
/// which matters for `[State -1]` entries and `[Command]` definitions.
#[derive(Debug, Clone, Default)]
pub struct TextFile {
    pub sections: Vec<Section>,
}

impl TextFile {
    pub fn parse_str(text: &str) -> Self {
        let mut file = TextFile::default();
        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                file.sections.push(Section {
                    header: inner.trim().to_string(),
                    line: line_no,
                    entries: Vec::new(),
                });
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = unquote(value.trim());
            if file.sections.is_empty() {
                file.sections.push(Section {
                    header: String::new(),
                    line: line_no,
                    entries: Vec::new(),
                });
            }
            file.sections.last_mut().unwrap().entries.push(Entry {
                key: key.trim().to_string(),
                value,
                line: line_no,
            });
        }
        file
    }

    pub fn parse_bytes(bytes: &[u8]) -> Self {
        let text = decode_text(bytes);
        Self::parse_str(&text)
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
        Ok(Self::parse_bytes(&bytes))
    }

    pub fn find(&self, kind: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| s.kind().eq_ignore_ascii_case(kind))
    }

    pub fn find_header(&self, header: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| s.header.eq_ignore_ascii_case(header))
    }
}

/// Removes a `;` comment while respecting double-quoted strings.
pub fn strip_comment(line: &str) -> &str {
    let mut in_quotes = false;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Trims whitespace and removes one layer of surrounding double quotes.
pub fn unquote(value: &str) -> String {
    let v = value.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

/// Decodes content files, which may be UTF-8 (with BOM), UTF-16LE, or legacy
/// single-byte encodings. Invalid sequences are replaced instead of failing.
pub fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Splits a comma-separated parameter list, respecting quoted strings.
pub fn split_params(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in value.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ',' if !in_quotes => {
                out.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() || !out.is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

/// Parses `x, y` or `x` into a pair.
pub fn parse_pair(value: &str) -> (f32, f32) {
    let parts = split_params(value);
    let x = parts
        .first()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0.0);
    let y = parts
        .get(1)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(x);
    (x, y)
}

/// Parses an integer pair such as `0, 1`.
pub fn parse_pair_i32(value: &str) -> (i32, i32) {
    let parts = split_params(value);
    let x = parts
        .first()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let y = parts
        .get(1)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(x);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_comments_quotes_and_duplicate_sections() {
        let txt = "\u{feff}[Info]\nname = \"Kung; Fu Man\" ; comment\n[State 0]\ntype=velset\n[State 0]\ntype=veladd\n";
        let f = TextFile::parse_str(txt);
        assert_eq!(f.sections.len(), 3);
        assert_eq!(f.sections[0].get("name").unwrap(), "Kung; Fu Man");
        assert_eq!(f.sections[1].kind(), "state");
        assert_eq!(f.sections[2].get("type").unwrap(), "veladd");
    }

    #[test]
    fn split_respects_quotes() {
        let p = split_params("-10, -76, \"a,b\"");
        assert_eq!(p, vec!["-10", "-76", "\"a,b\""]);
    }
}
