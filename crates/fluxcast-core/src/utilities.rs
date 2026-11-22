use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;
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
    let (command, input) = query.split_once(char::is_whitespace).unwrap_or((query, ""));
    let (label, answer) = match command {
        "b64" => ("Base64", Ok(STANDARD.encode(input))),
        "unb64" => (
            "Texto UTF-8",
            STANDARD
                .decode(input.trim())
                .map_err(|_| "Base64 inválido")
                .and_then(|v| String::from_utf8(v).map_err(|_| "El contenido no es UTF-8")),
        ),
        "hex" | "bin" | "dec" => ("Conversión de base", convert_number(command, input.trim())),
        "ts" => ("Fecha UTC / Unix", timestamp(input.trim())),
        "jwt" => ("JWT · contenido sin verificar", jwt(input.trim())),
        "uuid4" | "uuid7" if input.trim().is_empty() => (
            "UUID",
            Ok(if command == "uuid4" {
                Uuid::new_v4()
            } else {
                Uuid::now_v7()
            }
            .to_string()),
        ),
        _ if query.starts_with('=') => ("Calculadora", calculate(query[1..].trim())),
        _ if query.bytes().any(|b| b.is_ascii_digit()) && arithmetic_text(query) => {
            ("Calculadora", calculate(query))
        }
        _ => return None,
    };
    Some(answer.map(|value| Output { label, value }))
}

fn arithmetic_text(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_digit() || b"+-*/%^(). \t".contains(&b))
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
    if let Ok(seconds) = input.parse::<i64>() {
        return DateTime::<Utc>::from_timestamp(seconds, 0)
            .map(|date| date.to_rfc3339_opts(SecondsFormat::Secs, true))
            .ok_or("Timestamp fuera de rango");
    }
    DateTime::parse_from_rfc3339(input)
        .map(|date| date.timestamp().to_string())
        .map_err(|_| "Usa segundos Unix o una fecha ISO 8601 con zona horaria")
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
