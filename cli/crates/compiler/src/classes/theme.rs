use super::{Catalog, Offset, Rgb, Value};
use std::path::Path;
use toml::de::{DeTable, DeValue};

#[derive(Debug)]
pub(crate) struct ThemeError {
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub message: String,
}
fn error(path: &Path, text: &str, byte: usize, message: impl Into<String>) -> ThemeError {
    let prefix = &text[..byte.min(text.len())];
    ThemeError {
        path: path.to_string_lossy().replace('\\', "/"),
        line: prefix.bytes().filter(|b| *b == b'\n').count() as u32 + 1,
        column: prefix.rsplit('\n').next().unwrap_or("").len() as u32 + 1,
        message: message.into(),
    }
}
pub(super) fn load(dir: &Path) -> Result<Catalog, ThemeError> {
    let path = dir.join("theme.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&path, &text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Catalog::empty(false)),
        Err(e) => Err(error(&path, "", 0, format!("cannot read theme.toml: {e}"))),
    }
}
fn name(text: &str) -> bool {
    text.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && text.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}
fn numeric(value: &DeValue<'_>) -> Option<f64> {
    match value {
        DeValue::Integer(n) => i64::from_str_radix(n.as_str(), n.radix()).ok().map(|n| n as f64),
        DeValue::Float(n) => n.as_str().parse().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}
pub(super) fn parse(path: &Path, text: &str) -> Result<Catalog, ThemeError> {
    let document = DeTable::parse(text).map_err(|e| error(path, text, e.span().map_or(0, |s| s.start), e.message()))?;
    let mut catalog = Catalog::empty(true);
    let mut insert = |token: String, values: Vec<Value>, at: usize| -> Result<(), ThemeError> {
        if catalog.named.insert(token.clone(), values).is_some() {
            return Err(error(path, text, at, format!("two definitions make class `{token}`")));
        }
        Ok(())
    };
    let mut spacing = None;
    for (section, value) in document.get_ref() {
        let section_name = section.get_ref().as_ref();
        let bad = |message: &str| error(path, text, value.span().start, message);
        if section_name == "spacing" {
            spacing = Some(
                numeric(value.get_ref())
                    .filter(|n| *n > 0.0 && *n <= 1_000_000.0)
                    .ok_or_else(|| bad("spacing must be a positive finite number at most 1000000"))?,
            );
            continue;
        }
        if !matches!(section_name, "color" | "text" | "radius") {
            return Err(error(
                path,
                text,
                section.span().start,
                format!("unknown section `{section_name}`; expected spacing, color, text or radius"),
            ));
        }
        let DeValue::Table(table) = value.get_ref() else {
            return Err(bad("expected a table"));
        };
        for (key, value) in table {
            let key_name = key.get_ref().as_ref();
            if !name(key_name) && !(section_name == "radius" && key_name == "DEFAULT") {
                return Err(error(
                    path,
                    text,
                    key.span().start,
                    "a name must start with a lowercase letter and contain only lowercase letters and digits",
                ));
            }
            if section_name == "color" {
                let mut color =
                    |suffix: String, value: &DeValue<'_>, at: usize, key_at: usize| -> Result<(), ThemeError> {
                        let rgb = match value {
                            DeValue::String(s)
                                if s.len() == 7
                                    && s.starts_with('#')
                                    && s[1..].bytes().all(|b| b.is_ascii_hexdigit()) =>
                            {
                                Rgb(
                                    u8::from_str_radix(&s[1..3], 16).unwrap(),
                                    u8::from_str_radix(&s[3..5], 16).unwrap(),
                                    u8::from_str_radix(&s[5..7], 16).unwrap(),
                                )
                            }
                            _ => return Err(error(path, text, at, "a color must be #rrggbb")),
                        };
                        for (prefix, values) in [
                            ("bg", vec![Value::BackgroundColor3(rgb), Value::BackgroundTransparency(0.0)]),
                            ("text", vec![Value::TextColor3(rgb), Value::TextTransparency(0.0)]),
                            ("border", vec![Value::StrokeColor(rgb), Value::StrokeTransparency(0.0)]),
                        ] {
                            insert(format!("{prefix}-{suffix}"), values, key_at)?;
                        }
                        Ok(())
                    };
                if let DeValue::Table(shades) = value.get_ref() {
                    for (shade, v) in shades {
                        let shade_name = shade.get_ref().as_ref();
                        if matches!(v.get_ref(), DeValue::Table(_)) {
                            return Err(error(
                                path,
                                text,
                                shade.span().start,
                                "color tables allow only one level of shades",
                            ));
                        }
                        if shade_name.is_empty()
                            || !shade_name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                        {
                            return Err(error(
                                path,
                                text,
                                shade.span().start,
                                "a shade must contain lowercase letters or digits",
                            ));
                        }
                        color(format!("{key_name}-{shade_name}"), v.get_ref(), v.span().start, shade.span().start)?;
                    }
                } else {
                    color(key_name.into(), value.get_ref(), value.span().start, key.span().start)?;
                }
            } else {
                let n = numeric(value.get_ref())
                    .ok_or_else(|| error(path, text, value.span().start, "expected a finite number"))?;
                let (token, write) = if section_name == "text" {
                    if !(1.0..=100.0).contains(&n) {
                        return Err(error(path, text, value.span().start, "text size must be from 1 to 100 pixels"));
                    }
                    (format!("text-{key_name}"), Value::TextSize(n))
                } else {
                    if n < 0.0 || n > i32::MAX as f64 || n.fract() != 0.0 {
                        return Err(error(
                            path,
                            text,
                            value.span().start,
                            "radius must be a nonnegative whole-pixel offset in range",
                        ));
                    }
                    (
                        if key_name == "DEFAULT" { "rounded".into() } else { format!("rounded-{key_name}") },
                        Value::CornerRadius(Offset::Pixels(n as i32)),
                    )
                };
                insert(token, vec![write], key.span().start)?;
            }
        }
    }
    catalog.spacing = spacing.and_then(|n: f64| super::Decimal::parse(&n.to_string()));
    Ok(catalog)
}
