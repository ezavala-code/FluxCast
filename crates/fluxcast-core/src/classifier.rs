use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Text,
    Code,
    Json,
    Url,
    Color,
    Jwt,
}

impl ContentType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Code => "code",
            Self::Json => "json",
            Self::Url => "url",
            Self::Color => "color",
            Self::Jwt => "jwt",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "code" => Self::Code,
            "json" => Self::Json,
            "url" => Self::Url,
            "color" => Self::Color,
            "jwt" => Self::Jwt,
            _ => Self::Text,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Classification {
    pub content_type: ContentType,
    pub preview: String,
    pub formatted: Option<String>,
    pub language: Option<&'static str>,
    pub color_swatch: Option<String>,
}

pub fn classify(content: &str) -> Classification {
    let trimmed = content.trim();
    let preview = build_preview(trimmed);

    if let Some(decoded_jwt) = detect_jwt(trimmed) {
        return Classification {
            content_type: ContentType::Jwt,
            preview,
            formatted: Some(decoded_jwt),
            language: Some("json"),
            color_swatch: None,
        };
    }

    if let Some(swatch) = detect_color(trimmed) {
        return Classification {
            content_type: ContentType::Color,
            preview,
            formatted: None,
            language: None,
            color_swatch: Some(swatch),
        };
    }

    if let Some(pretty_json) = detect_json(trimmed) {
        return Classification {
            content_type: ContentType::Json,
            preview,
            formatted: Some(pretty_json),
            language: Some("json"),
            color_swatch: None,
        };
    }

    if detect_url(trimmed) {
        return Classification {
            content_type: ContentType::Url,
            preview,
            formatted: None,
            language: None,
            color_swatch: None,
        };
    }

    if let Some(lang) = detect_code_language(trimmed) {
        return Classification {
            content_type: ContentType::Code,
            preview,
            formatted: None,
            language: Some(lang),
            color_swatch: None,
        };
    }

    Classification {
        content_type: ContentType::Text,
        preview,
        formatted: None,
        language: None,
        color_swatch: None,
    }
}

pub fn build_preview(content: &str) -> String {
    let collapsed = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let source = if collapsed.is_empty() {
        content
    } else {
        &collapsed
    };
    source.chars().take(240).collect()
}

/// HMAC/digest determinístico para deduplicación local sin exponer texto plano en índices.
pub fn dedup_hmac(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"fluxcast-dedup-v1:");
    hasher.update(content.trim().as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

/// Limpia formato, secuencias ANSI y normaliza saltos de línea para pegado limpio (`Shift+Enter`).
pub fn plain_text(content: &str) -> String {
    let stripped = strip_ansi_codes(content);
    stripped
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_owned()
}

fn strip_ansi_codes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn detect_jwt(trimmed: &str) -> Option<String> {
    if trimmed.contains(char::is_whitespace) {
        return None;
    }
    let parts: Vec<_> = trimmed.split('.').collect();
    if parts.len() != 3 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }
    let decode = |segment: &str| -> Option<Value> {
        let bytes = URL_SAFE_NO_PAD.decode(segment).ok()?;
        let value: Value = serde_json::from_slice(&bytes).ok()?;
        value.is_object().then_some(value)
    };
    let header = decode(parts[0])?;
    let payload = decode(parts[1])?;
    let doc = serde_json::json!({
        "header": header,
        "payload": payload,
    });
    serde_json::to_string_pretty(&doc).ok()
}

fn detect_json(trimmed: &str) -> Option<String> {
    let is_compound = (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'));
    if !is_compound {
        return None;
    }
    let value: Value = serde_json::from_str(trimmed).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

fn detect_url(trimmed: &str) -> bool {
    if trimmed.contains(char::is_whitespace) {
        return false;
    }
    ["https://", "http://", "mailto:", "ssh://"]
        .iter()
        .any(|prefix| {
            trimmed
                .get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
                && trimmed.len() > prefix.len() + 2
        })
}

fn detect_color(trimmed: &str) -> Option<String> {
    if let Some(hex) = trimmed.strip_prefix('#') {
        if matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(format!("#{}", hex.to_ascii_lowercase()));
        }
    }
    let lower = trimmed.to_ascii_lowercase();
    let inner = lower
        .strip_prefix("rgb(")
        .or_else(|| lower.strip_prefix("rgba("))
        .and_then(|s| s.strip_suffix(')'))?;
    let parts: Vec<_> = inner.split(',').map(str::trim).collect();
    if !matches!(parts.len(), 3 | 4) {
        return None;
    }
    for &channel in &parts[..3] {
        let value: u16 = channel.parse().ok()?;
        if value > 255 {
            return None;
        }
    }
    if let Some(&alpha) = parts.get(3) {
        let a: f32 = alpha.parse().ok()?;
        if !(0.0..=1.0).contains(&a) {
            return None;
        }
    }
    Some(lower)
}

fn detect_code_language(trimmed: &str) -> Option<&'static str> {
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.starts_with("#!/")
        || [
            "sudo dnf ",
            "docker ",
            "podman ",
            "kubectl ",
            "git ",
            "cargo ",
            "systemctl ",
        ]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        return Some("shell");
    }
    if [
        "fn ",
        "pub fn ",
        "let mut ",
        "impl ",
        "use std::",
        "#[derive(",
    ]
    .iter()
    .any(|k| trimmed.contains(k))
    {
        return Some("rust");
    }
    if ["fun ", "val ", "var ", "@Composable", "package dev."]
        .iter()
        .any(|k| trimmed.contains(k))
    {
        return Some("kotlin");
    }
    if [
        "import ",
        "export ",
        "const ",
        "function ",
        "interface ",
        "=>",
        "console.log(",
    ]
    .iter()
    .any(|k| trimmed.contains(k))
    {
        return Some("typescript");
    }
    if [
        "select ",
        "insert into ",
        "create table ",
        "update ",
        "delete from ",
    ]
    .iter()
    .any(|k| lower.starts_with(k) || lower.contains(&format!("\n{k}")))
    {
        return Some("sql");
    }
    if ["def ", "import ", "from ", "if __name__ =="]
        .iter()
        .any(|k| trimmed.contains(k))
    {
        return Some("python");
    }
    let lines: Vec<_> = trimmed.lines().collect();
    if lines.len() >= 2
        && lines
            .iter()
            .filter(|l| {
                let t = l.trim_end();
                t.ends_with(';') || t.ends_with('{') || t.ends_with('}')
            })
            .count()
            >= 2
    {
        return Some("code");
    }
    None
}
