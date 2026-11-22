use chrono::{DateTime, Utc};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Expande variables dinámicas en plantillas de snippets:
/// - `{{clipboard}}`
/// - `{{uuid}}` / `{{uuid4}}`
/// - `{{uuid7}}`
/// - `{{date}}` (por defecto `YYYY-MM-DD`)
/// - `{{date:FORMATO}}` (soporta tokens `YYYY`, `MM`, `DD`, `HH`, `mm`, `ss` o `strftime`)
pub fn expand(template: &str, clipboard: &str) -> String {
    let now = current_utc();
    expand_at(template, clipboard, now)
}

pub fn expand_at(template: &str, clipboard: &str, now: DateTime<Utc>) -> String {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            output.push_str(&rest[start..]);
            return output;
        };
        let token = after_open[..end].trim();
        match token {
            "clipboard" => output.push_str(clipboard),
            "uuid" | "uuid4" => output.push_str(&Uuid::new_v4().to_string()),
            "uuid7" => output.push_str(&Uuid::now_v7().to_string()),
            "date" => output.push_str(&now.format("%Y-%m-%d").to_string()),
            _ if token.starts_with("date:") => {
                let pattern = token["date:".len()..].trim();
                let strftime_pattern = normalize_date_pattern(pattern);
                output.push_str(&now.format(&strftime_pattern).to_string());
            }
            _ => {
                output.push_str("{{");
                output.push_str(&after_open[..end]);
                output.push_str("}}");
            }
        }
        rest = &after_open[end + 2..];
    }
    output.push_str(rest);
    output
}

fn current_utc() -> DateTime<Utc> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    DateTime::<Utc>::from_timestamp(secs, 0).unwrap_or_default()
}

fn normalize_date_pattern(pattern: &str) -> String {
    if pattern.is_empty() {
        return "%Y-%m-%d".into();
    }
    pattern
        .replace("YYYY", "%Y")
        .replace("MM", "%m")
        .replace("DD", "%d")
        .replace("HH", "%H")
        .replace("mm", "%M")
        .replace("ss", "%S")
}
