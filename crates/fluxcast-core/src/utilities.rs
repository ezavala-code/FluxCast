use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct Output {
    pub label: &'static str,
    pub value: String,
}

type Answer = Result<String, &'static str>;

/// El resultado se calcula una sola vez y la interfaz copia ese mismo valor.
pub fn evaluate(query: &str) -> Option<Result<Output, &'static str>> {
    let query = query.trim_start();
    if query.is_empty() {
        return None;
    }
    if let Some(expr) = query.strip_prefix('=') {
        return Some(calculate(expr.trim()).map(|value| Output {
            label: "Calculadora",
            value,
        }));
    }
    let (raw_command, input) = match query.split_once(char::is_whitespace) {
        Some((command, rest)) => (command, Some(rest)),
        None => (query, None),
    };
    let command = raw_command.to_ascii_lowercase();
    let (label, answer) = match (command.as_str(), input) {
        ("uuid4", None) => ("UUID", Ok(Uuid::new_v4().to_string())),
        ("uuid7", None) => ("UUID", Ok(Uuid::now_v7().to_string())),
        ("uuid4", Some(rest)) if rest.trim().is_empty() => ("UUID", Ok(Uuid::new_v4().to_string())),
        ("uuid7", Some(rest)) if rest.trim().is_empty() => ("UUID", Ok(Uuid::now_v7().to_string())),
        ("b64", Some(rest)) if !rest.is_empty() => ("Base64", Ok(STANDARD.encode(rest))),
        ("unb64", Some(rest)) => ("Texto UTF-8", decode_base64(rest.trim())),
        ("hex" | "bin" | "dec", Some(rest)) => {
            ("Conversión de base", convert_number(&command, rest.trim()))
        }
        ("ts", Some(rest)) => ("Fecha UTC / Unix", timestamp(rest.trim())),
        ("jwt", Some(rest)) => ("JWT · contenido sin verificar", jwt(rest.trim())),
        _ if is_implicit_math(query) => ("Calculadora", calculate(query)),
        _ => return None,
    };
    Some(answer.map(|value| Output { label, value }))
}

fn arithmetic_text(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_digit() || b"+-*/%^(). \t".contains(&b))
}

fn is_implicit_math(text: &str) -> bool {
    text.bytes().any(|b| b.is_ascii_digit())
        && text.bytes().any(|b| b"+-*/%^".contains(&b))
        && arithmetic_text(text)
}

fn decode_base64(input: &str) -> Answer {
    if input.is_empty() {
        return Err("Escribe texto Base64 para decodificar");
    }
    STANDARD
        .decode(input)
        .map_err(|_| "Base64 inválido")
        .and_then(|v| String::from_utf8(v).map_err(|_| "El contenido no es UTF-8"))
}

fn calculate(text: &str) -> Answer {
    if !arithmetic_text(text) {
        return Err("Usa números, paréntesis y operadores aritméticos");
    }
    let answer = fasteval::ez_eval(text, &mut fasteval::EmptyNamespace)
        .map_err(|_| "Expresión incompleta o inválida")?;
    answer
        .is_finite()
        .then(|| answer.to_string())
        .ok_or("El resultado no es finito")
}

fn convert_number(command: &str, input: &str) -> Answer {
    let (sign, digits) = input.strip_prefix('-').map_or(("", input), |v| ("-", v));
    let (base, digits) = if let Some(v) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        (16, v)
    } else if let Some(v) = digits
        .strip_prefix("0b")
        .or_else(|| digits.strip_prefix("0B"))
    {
        (2, v)
    } else {
        (10, digits)
    };
    let value = u128::from_str_radix(digits, base)
        .map_err(|_| "Usa decimal, 0x hexadecimal o 0b binario")?;
    Ok(match command {
        "hex" => format!("{sign}0x{value:X}"),
        "bin" => format!("{sign}0b{value:b}"),
        _ => format!("{sign}{value}"),
    })
}

fn timestamp(input: &str) -> Answer {
    if input.eq_ignore_ascii_case("now") {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "Reloj del sistema anterior a 1970")?
            .as_secs();
        return Ok(secs.to_string());
    }
    if let Ok(seconds) = input.parse::<i64>() {
        return DateTime::<Utc>::from_timestamp(seconds, 0)
            .map(|date| date.to_rfc3339_opts(SecondsFormat::Secs, true))
            .ok_or("Timestamp fuera de rango");
    }
    DateTime::parse_from_rfc3339(input)
        .map(|date| date.timestamp().to_string())
        .map_err(|_| "Usa 'now', segundos Unix o una fecha ISO 8601 con zona horaria")
}

fn jwt(token: &str) -> Answer {
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("El JWT debe contener tres segmentos");
    }
    let decode = |s| -> Result<Value, &'static str> {
        let bytes = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| "Segmento Base64URL inválido")?;
        let obj: Value = serde_json::from_slice(&bytes).map_err(|_| "JSON inválido en el JWT")?;
        if !obj.is_object() {
            return Err("Se esperaba un objeto JSON en el JWT");
        }
        Ok(obj)
    };
    let document = serde_json::json!({"header": decode(parts[0])?, "payload": decode(parts[1])?});
    Ok(serde_json::to_string_pretty(&document).expect("JSON serializable"))
}
