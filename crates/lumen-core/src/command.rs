//! ClickMonitorDDC-compatible command language.
//!
//! ```text
//! PL2779Q b 36 HPA34T c - 5     brightness 36 on PL2779Q, contrast -5 on HPA34T
//! MateView b-10 N2724A b+10     compact form
//! t b 0 c 0 t b 90 c 80         alternate between two programs on every run
//! setVCP 0x10 + 10              raw VCP access
//! ```

use serde::Serialize;

use crate::vcp::{self, Feature};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Value {
    Abs(u32),
    Rel(i32),
}

impl Value {
    pub fn apply(self, current: u32, min: u32, max: u32) -> u32 {
        let v = match self {
            Value::Abs(v) => v as i64,
            Value::Rel(d) => current as i64 + d as i64,
        };
        v.clamp(min as i64, max as i64) as u32
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum Step {
    /// Selects the monitor(s) for the following steps (name, model, serial, alias, id or number).
    Select { monitor: String },
    Adjust { feature: Feature, value: Value },
    ColorPreset { name: String },
    DisplayMode { name: String },
    Input { name: String },
    Saturation { value: Value },
    SystemVolume { value: Value },
    Mute,
    Unmute,
    Dimming { enabled: bool },
    SourceBrightness { enabled: bool },
    Notify,
    Rotate,
    /// `x`: without a selector all monitors go to power-save, with one it toggles that monitor via VCP 0xD6.
    Power,
    FactoryReset,
    Pause { ms: u64 },
    Quit,
    Restart,
    Run { program: String },
    ReturnBrightness,
    ReturnContrast,
    SetVcp { code: u8, value: Value },
    GetVcp { code: u8 },
    ReturnVcp { code: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Program {
    Steps(Vec<Step>),
    /// `t`-separated alternatives; one is executed per run, in turn.
    Toggle(Vec<Vec<Step>>),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("'{0}' expects a value")]
    MissingValue(String),
    #[error("invalid value '{1}' for '{0}'")]
    InvalidValue(String, String),
    #[error("command line is longer than {0} characters")]
    TooLong(usize),
}

pub const MAX_LEN: usize = 4096;

pub fn parse(line: &str) -> Result<Program, ParseError> {
    if line.len() > MAX_LEN {
        return Err(ParseError::TooLong(MAX_LEN));
    }
    let tokens = tokenize(line);
    if tokens.iter().any(|t| t.eq_ignore_ascii_case("t")) {
        let mut groups: Vec<Vec<String>> = vec![Vec::new()];
        for t in tokens {
            if t.eq_ignore_ascii_case("t") {
                groups.push(Vec::new());
            } else {
                groups.last_mut().unwrap().push(t);
            }
        }
        let alternatives = groups
            .into_iter()
            .filter(|g| !g.is_empty())
            .map(|g| parse_tokens(&g))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Program::Toggle(alternatives))
    } else {
        Ok(Program::Steps(parse_tokens(&tokens)?))
    }
}

fn tokenize(line: &str) -> Vec<String> {
    let mut raw: Vec<String> = Vec::new();
    let mut chars = line.chars().peekable();
    let mut cur = String::new();
    let flush = |cur: &mut String, raw: &mut Vec<String>| {
        if !cur.is_empty() {
            raw.push(std::mem::take(cur));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '#' => break,
            '"' => {
                flush(&mut cur, &mut raw);
                let quoted: String = chars.by_ref().take_while(|&c| c != '"').collect();
                raw.push(quoted.split_whitespace().collect::<String>());
            }
            ';' | ',' => {
                flush(&mut cur, &mut raw);
                raw.push(c.to_string());
            }
            '!' if cur.is_empty() => {
                flush(&mut cur, &mut raw);
                let rest: String = chars.by_ref().collect();
                raw.push("!".into());
                raw.push(rest.trim().to_string());
            }
            c if c.is_whitespace() => flush(&mut cur, &mut raw),
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut raw);

    let mut out = Vec::with_capacity(raw.len());
    for tok in raw {
        if let Some((cmd, rest)) = split_compact(&tok) {
            out.push(cmd.to_string());
            push_signed(&mut out, rest);
        } else if tok.len() > 1 && (tok.starts_with('+') || tok.starts_with('-')) {
            push_signed(&mut out, &tok);
        } else {
            out.push(tok);
        }
    }
    out
}

fn push_signed(out: &mut Vec<String>, s: &str) {
    if let Some(rest) = s.strip_prefix('+') {
        out.push("+".into());
        if !rest.is_empty() {
            out.push(rest.into());
        }
    } else if let Some(rest) = s.strip_prefix('-') {
        out.push("-".into());
        if !rest.is_empty() {
            out.push(rest.into());
        }
    } else {
        out.push(s.into());
    }
}

const VALUE_COMMANDS: &[&str] = &["bl", "b", "c", "r", "g", "l", "i", "k", "o", "v", "n", "y"];

/// `b-10` -> (`b`, `-10`), `bl250` -> (`bl`, `250`). Monitor names like `N2724A` are left alone.
fn split_compact(tok: &str) -> Option<(&str, &str)> {
    let lower = tok.to_ascii_lowercase();
    for cmd in VALUE_COMMANDS {
        if let Some(rest) = lower.strip_prefix(cmd) {
            let digits = rest.strip_prefix(['+', '-']).unwrap_or(rest);
            if !rest.is_empty() && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                || rest == "+"
                || rest == "-"
            {
                return Some((&tok[..cmd.len()], &tok[cmd.len()..]));
            }
        }
    }
    None
}

struct Cursor<'a> {
    toks: &'a [String],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn next(&mut self) -> Option<&'a str> {
        let t = self.toks.get(self.i)?;
        self.i += 1;
        Some(t.as_str())
    }

    fn value(&mut self, cmd: &str) -> Result<Value, ParseError> {
        let t = self.next().ok_or_else(|| ParseError::MissingValue(cmd.into()))?;
        let num = |s: &str| -> Result<u32, ParseError> {
            vcp::parse_number(s).ok_or_else(|| ParseError::InvalidValue(cmd.into(), s.into()))
        };
        match t {
            "+" | "-" => {
                let n = self.next().ok_or_else(|| ParseError::MissingValue(cmd.into()))?;
                let n = num(n)? as i32;
                Ok(Value::Rel(if t == "-" { -n } else { n }))
            }
            _ => Ok(Value::Abs(num(t)?)),
        }
    }

    fn word(&mut self, cmd: &str) -> Result<String, ParseError> {
        self.next()
            .map(str::to_string)
            .ok_or_else(|| ParseError::MissingValue(cmd.into()))
    }

    fn flag(&mut self, cmd: &str) -> Result<bool, ParseError> {
        match self.next() {
            Some("0") => Ok(false),
            Some("1") => Ok(true),
            Some(other) => Err(ParseError::InvalidValue(cmd.into(), other.into())),
            None => Err(ParseError::MissingValue(cmd.into())),
        }
    }

    fn code(&mut self, cmd: &str) -> Result<u8, ParseError> {
        let t = self.next().ok_or_else(|| ParseError::MissingValue(cmd.into()))?;
        vcp::parse_number(t)
            .and_then(|n| u8::try_from(n).ok())
            .ok_or_else(|| ParseError::InvalidValue(cmd.into(), t.into()))
    }
}

fn parse_tokens(toks: &[String]) -> Result<Vec<Step>, ParseError> {
    let mut c = Cursor { toks, i: 0 };
    let mut steps = Vec::new();
    while let Some(tok) = c.next() {
        let lower = tok.to_ascii_lowercase();
        let step = match lower.as_str() {
            "b" => Step::Adjust { feature: Feature::Brightness, value: c.value(tok)? },
            "c" => Step::Adjust { feature: Feature::Contrast, value: c.value(tok)? },
            "r" => Step::Adjust { feature: Feature::Red, value: c.value(tok)? },
            "g" => Step::Adjust { feature: Feature::Green, value: c.value(tok)? },
            "l" | "bl" | "i" => Step::Adjust { feature: Feature::Blue, value: c.value(tok)? },
            "o" => Step::Adjust { feature: Feature::Volume, value: c.value(tok)? },
            "k" => Step::Saturation { value: c.value(tok)? },
            "v" => Step::SystemVolume { value: c.value(tok)? },
            "p" => Step::ColorPreset { name: c.word(tok)? },
            "j" => Step::DisplayMode { name: c.word(tok)? },
            "s" => Step::Input { name: c.word(tok)? },
            "n" => Step::Dimming { enabled: c.flag(tok)? },
            "y" => Step::SourceBrightness { enabled: c.flag(tok)? },
            "m" => Step::Mute,
            "u" => Step::Unmute,
            "?" => Step::Notify,
            "w" => Step::Rotate,
            "x" => Step::Power,
            "f" => Step::FactoryReset,
            ";" => Step::Pause { ms: 100 },
            "," => Step::Pause { ms: 1000 },
            "q" => Step::Quit,
            "z" => Step::Restart,
            "d" => Step::ReturnBrightness,
            "e" => Step::ReturnContrast,
            "!" => Step::Run { program: c.word(tok)? },
            "setvcp" => {
                let code = c.code(tok)?;
                Step::SetVcp { code, value: c.value(tok)? }
            }
            "getvcp" => Step::GetVcp { code: c.code(tok)? },
            "get_vcp" => Step::ReturnVcp { code: c.code(tok)? },
            _ => Step::Select { monitor: tok.to_string() },
        };
        steps.push(step);
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Step::*;

    fn steps(s: &str) -> Vec<Step> {
        match parse(s).unwrap() {
            Program::Steps(v) => v,
            p => panic!("unexpected {p:?}"),
        }
    }

    fn sel(m: &str) -> Step {
        Select { monitor: m.into() }
    }

    #[test]
    fn spaced_form() {
        assert_eq!(
            steps("PL2779Q b 36  HPA34T c - 5"),
            vec![
                sel("PL2779Q"),
                Adjust { feature: Feature::Brightness, value: Value::Abs(36) },
                sel("HPA34T"),
                Adjust { feature: Feature::Contrast, value: Value::Rel(-5) },
            ]
        );
    }

    #[test]
    fn compact_form() {
        assert_eq!(
            steps("MateView b-10 N2724A b+10"),
            vec![
                sel("MateView"),
                Adjust { feature: Feature::Brightness, value: Value::Rel(-10) },
                sel("N2724A"),
                Adjust { feature: Feature::Brightness, value: Value::Rel(10) },
            ]
        );
        assert_eq!(
            steps("b +5 bl250"),
            vec![
                Adjust { feature: Feature::Brightness, value: Value::Rel(5) },
                Adjust { feature: Feature::Blue, value: Value::Abs(250) },
            ]
        );
    }

    #[test]
    fn toggle_with_monitor_number() {
        assert_eq!(
            parse(" 1 b 0 t 1 b 50").unwrap(),
            Program::Toggle(vec![
                vec![sel("1"), Adjust { feature: Feature::Brightness, value: Value::Abs(0) }],
                vec![sel("1"), Adjust { feature: Feature::Brightness, value: Value::Abs(50) }],
            ])
        );
        match parse("t b 0 c 0 t b 90 c 80").unwrap() {
            Program::Toggle(v) => assert_eq!(v.len(), 2),
            p => panic!("{p:?}"),
        }
    }

    #[test]
    fn pauses_comments_and_run() {
        assert_eq!(
            steps("r 255 , g 255; bl 255 # ignored b 0"),
            vec![
                Adjust { feature: Feature::Red, value: Value::Abs(255) },
                Pause { ms: 1000 },
                Adjust { feature: Feature::Green, value: Value::Abs(255) },
                Pause { ms: 100 },
                Adjust { feature: Feature::Blue, value: Value::Abs(255) },
            ]
        );
        assert_eq!(steps("b 10 ! notepad.exe"), vec![
            Adjust { feature: Feature::Brightness, value: Value::Abs(10) },
            Run { program: "notepad.exe".into() },
        ]);
    }

    #[test]
    fn vcp_and_misc() {
        assert_eq!(
            steps("setVCP 0x10 + 10 getVCP 0x12 get_VCP 18"),
            vec![
                SetVcp { code: 0x10, value: Value::Rel(10) },
                GetVcp { code: 0x12 },
                ReturnVcp { code: 0x12 },
            ]
        );
        assert_eq!(
            steps("\"HP w2207\" p 6500K s HDMI1 j Movie n 0 x"),
            vec![
                sel("HPw2207"),
                ColorPreset { name: "6500K".into() },
                Input { name: "HDMI1".into() },
                DisplayMode { name: "Movie".into() },
                Dimming { enabled: false },
                Power,
            ]
        );
    }

    #[test]
    fn errors() {
        assert_eq!(parse("b"), Err(ParseError::MissingValue("b".into())));
        assert_eq!(parse("b +"), Err(ParseError::MissingValue("b".into())));
        assert!(matches!(parse("b abc"), Err(ParseError::InvalidValue(..))));
    }

    #[test]
    fn value_apply_clamps() {
        assert_eq!(Value::Rel(-10).apply(5, 0, 100), 0);
        assert_eq!(Value::Rel(10).apply(95, 0, 100), 100);
        assert_eq!(Value::Abs(150).apply(0, 0, 100), 100);
        assert_eq!(Value::Abs(20).apply(0, 30, 100), 30);
    }
}
