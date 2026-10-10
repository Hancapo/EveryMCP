use num_bigint::BigInt;
use serde_json::Value;

pub fn parse(source: &str) -> Result<Value, String> {
    // shortcut: serde reserves this object key with arbitrary precision; reject until upstream preserves it.
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() && bytes[index] != b'"' {
            index += if bytes[index] == b'\\' { 2 } else { 1 };
        }
        if index >= bytes.len() {
            break;
        }
        index += 1;
        if source[index..].trim_start().starts_with(':')
            && serde_json::from_str::<String>(&source[start..index])
                .is_ok_and(|key| key == "$serde_json::private::Number")
        {
            return Err("JSON object key '$serde_json::private::Number' is unsupported.".into());
        }
    }
    serde_json::from_str(source).map_err(|e| format!("Invalid JSON: {e}"))
}

fn normalized_number(number: &serde_json::Number) -> (bool, String, BigInt) {
    let raw = number.as_str();
    let negative = raw.starts_with('-');
    let unsigned = raw.trim_start_matches('-');
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let fraction_len = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let digits = mantissa.replace('.', "");
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return (false, String::new(), BigInt::from(0));
    }
    let trimmed = significant.trim_end_matches('0');
    let exponent = exponent.parse::<BigInt>().expect("Validated JSON exponent")
        - BigInt::from(fraction_len)
        + BigInt::from(significant.len() - trimmed.len());
    (negative, trimmed.to_owned(), exponent)
}

pub fn equivalent(left: &Value, right: &Value) -> bool {
    if left == right {
        return true;
    }
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => normalized_number(a) == normalized_number(b),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equivalent(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| equivalent(a, b)))
        }
        _ => false,
    }
}
