//! Parser for MCCS capability strings, e.g.
//! `(prot(monitor)type(LCD)model(PL2779Q)cmds(01 02 03)vcp(02 04 10 12 14(05 08 0B) 60(0F 11 12))mccs_ver(2.2))`

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Capabilities {
    pub raw: String,
    pub model: Option<String>,
    pub kind: Option<String>,
    pub mccs_version: Option<String>,
    /// Supported VCP codes with their allowed discrete values (empty = continuous / unspecified).
    pub vcp: BTreeMap<u8, Vec<u32>>,
}

impl Capabilities {
    pub fn supports(&self, code: u8) -> bool {
        self.vcp.contains_key(&code)
    }

    pub fn values(&self, code: u8) -> &[u32] {
        self.vcp.get(&code).map(Vec::as_slice).unwrap_or(&[])
    }
}

pub fn parse(raw: &str) -> Capabilities {
    let mut caps = Capabilities {
        raw: raw.to_string(),
        ..Default::default()
    };
    let s = raw.trim().trim_matches(char::from(0));
    let s = s.strip_prefix('(').unwrap_or(s);
    let s = s.strip_suffix(')').unwrap_or(s);

    for (key, body) in top_level_groups(s) {
        match key.to_ascii_lowercase().as_str() {
            "model" => caps.model = Some(body.trim().to_string()),
            "type" => caps.kind = Some(body.trim().to_string()),
            "mccs_ver" => caps.mccs_version = Some(body.trim().to_string()),
            "vcp" => caps.vcp = parse_vcp(body),
            _ => {}
        }
    }
    caps
}

/// Splits `key(body)key2(body2)` honouring nested parentheses.
fn top_level_groups(s: &str) -> Vec<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'(' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key = s[key_start..i].trim();
        let body_start = i + 1;
        let mut depth = 1;
        i += 1;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        let body_end = if depth == 0 { i - 1 } else { i };
        out.push((key, &s[body_start..body_end]));
    }
    out
}

fn parse_vcp(body: &str) -> BTreeMap<u8, Vec<u32>> {
    let mut map = BTreeMap::new();
    let mut tokens = Tokens::new(body);
    let mut last: Option<u8> = None;
    while let Some(tok) = tokens.next() {
        match tok {
            Tok::Hex(h) => {
                if let Ok(code) = u8::try_from(h) {
                    map.entry(code).or_insert_with(Vec::new);
                    last = Some(code);
                }
            }
            Tok::Open => {
                let mut values = Vec::new();
                let mut depth = 1;
                while let Some(t) = tokens.next() {
                    match t {
                        Tok::Open => depth += 1,
                        Tok::Close => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        Tok::Hex(v) if depth == 1 => values.push(v),
                        Tok::Hex(_) => {}
                    }
                }
                if let Some(code) = last {
                    map.insert(code, values);
                }
            }
            Tok::Close => {}
        }
    }
    map
}

enum Tok {
    Hex(u32),
    Open,
    Close,
}

struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Tokens<'a> {
    fn new(s: &'a str) -> Self {
        Self { s: s.as_bytes(), i: 0 }
    }
}

impl Iterator for Tokens<'_> {
    type Item = Tok;
    fn next(&mut self) -> Option<Tok> {
        while self.i < self.s.len() {
            let c = self.s[self.i];
            match c {
                b'(' => {
                    self.i += 1;
                    return Some(Tok::Open);
                }
                b')' => {
                    self.i += 1;
                    return Some(Tok::Close);
                }
                c if c.is_ascii_hexdigit() => {
                    // Codes are written as pairs of hex digits, sometimes without separators.
                    let start = self.i;
                    let mut end = start;
                    while end < self.s.len() && self.s[end].is_ascii_hexdigit() && end - start < 2 {
                        end += 1;
                    }
                    self.i = end;
                    let text = std::str::from_utf8(&self.s[start..end]).ok()?;
                    return u32::from_str_radix(text, 16).ok().map(Tok::Hex);
                }
                _ => self.i += 1,
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_string() {
        let caps = parse(
            "(prot(monitor)type(LCD)model(PL2779Q)cmds(01 02 03 07 0C E3 F3)vcp(02 04 05 08 10 12 14(05 08 0B) 16 18 1A 60(0F 11 12) 62 D6(01 04 05) DC(00 03 05) DF)mccs_ver(2.2))",
        );
        assert_eq!(caps.model.as_deref(), Some("PL2779Q"));
        assert_eq!(caps.kind.as_deref(), Some("LCD"));
        assert_eq!(caps.mccs_version.as_deref(), Some("2.2"));
        assert!(caps.supports(0x10));
        assert_eq!(caps.values(0x60), &[0x0F, 0x11, 0x12]);
        assert_eq!(caps.values(0x14), &[0x05, 0x08, 0x0B]);
        assert_eq!(caps.values(0xD6), &[0x01, 0x04, 0x05]);
        assert!(caps.supports(0xDF));
    }

    #[test]
    fn parses_without_separators() {
        let caps = parse("(vcp(101214(050608)60(0F11)))");
        assert!(caps.supports(0x10));
        assert!(caps.supports(0x12));
        assert_eq!(caps.values(0x14), &[0x05, 0x06, 0x08]);
        assert_eq!(caps.values(0x60), &[0x0F, 0x11]);
    }
}
