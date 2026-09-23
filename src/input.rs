use num_bigint::BigInt;
use num_traits::ToPrimitive;
use serde_json::Value;

pub fn property<'a>(args: &'a Value, name: &str) -> Result<&'a Value, String> {
    args.as_object()
        .and_then(|obj| obj.get(name))
        .ok_or_else(|| format!("Missing required argument '{name}'."))
}

pub fn number(args: &Value, name: &str) -> Result<f64, String> {
    let value = property(args, name)?;
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("'{name}' must be a finite number."))
}

pub fn int(args: &Value, name: &str) -> Result<i32, String> {
    let value = property(args, name)?;
    value
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| format!("'{name}' must be an integer."))
}

pub fn string<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    let value = property(args, name)?;
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("'{name}' must be a nonempty string."))
}

pub fn optional_bool(args: &Value, name: &str) -> Result<bool, String> {
    match args.get(name) {
        None => Ok(false),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("'{name}' must be a boolean.")),
    }
}

pub fn boolean(args: &Value, name: &str) -> Result<bool, String> {
    property(args, name)?
        .as_bool()
        .ok_or_else(|| format!("'{name}' must be a boolean."))
}

pub fn vector(args: &Value, name: &str, lengths: &[usize]) -> Result<Vec<f64>, String> {
    let values = property(args, name)?
        .as_array()
        .ok_or_else(|| format!("'{name}' must be an array."))?;
    let numbers: Result<Vec<_>, _> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("'{name}[{index}]' must be a finite number."))
        })
        .collect();
    let numbers = numbers?;
    if !lengths.is_empty() && !lengths.contains(&numbers.len()) {
        return Err(format!(
            "'{name}' must contain {} numbers.",
            lengths
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(" or ")
        ));
    }
    Ok(numbers)
}

pub fn integer(args: &Value, name: &str) -> Result<i128, String> {
    big_integer(args, name)?
        .to_i128()
        .ok_or_else(|| format!("'{name}' is not a valid integer."))
}

pub fn big_integer(args: &Value, name: &str) -> Result<BigInt, String> {
    parse_big_integer(string(args, name)?, name)
}

pub fn parse_big_integer(raw: &str, name: &str) -> Result<BigInt, String> {
    let raw = raw.trim();
    let (negative, text) = raw
        .strip_prefix('-')
        .map_or((false, raw), |tail| (true, tail));
    let invalid = || format!("'{name}' is not a valid integer.");
    if text.is_empty() {
        return Err(invalid());
    }
    let (radix, digits) =
        if let Some(rest) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            (16, rest)
        } else if let Some(rest) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
            (2, rest)
        } else {
            (10, text)
        };
    if digits.is_empty() {
        return Err(invalid());
    }
    let magnitude = BigInt::parse_bytes(digits.as_bytes(), radix).ok_or_else(invalid)?;
    Ok(if negative { -magnitude } else { magnitude })
}

pub fn optional_number(args: &Value, name: &str) -> Result<Option<f64>, String> {
    if args.get(name).is_some() {
        number(args, name).map(Some)
    } else {
        Ok(None)
    }
}

pub fn optional_int(args: &Value, name: &str) -> Result<Option<i32>, String> {
    if args.get(name).is_some() {
        int(args, name).map(Some)
    } else {
        Ok(None)
    }
}

pub fn matrix(args: &Value, name: &str, max_size: usize) -> Result<Vec<Vec<f64>>, String> {
    let rows = property(args, name)?
        .as_array()
        .ok_or_else(|| format!("'{name}' must be an array."))?;
    if rows.is_empty() || rows.len() > max_size {
        return Err(format!("'{name}' must contain 1 to {max_size} rows."));
    }
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            vector(&serde_json::json!({"row":row}), "row", &[rows.len()])
                .map_err(|message| format!("'{name}[{index}]': {message}"))
        })
        .collect()
}

pub fn points(
    args: &Value,
    name: &str,
    dimensions: usize,
    minimum: usize,
    maximum: usize,
) -> Result<Vec<Vec<f64>>, String> {
    let rows = property(args, name)?
        .as_array()
        .ok_or_else(|| format!("'{name}' must be an array."))?;
    if rows.len() < minimum || rows.len() > maximum {
        return Err(format!(
            "'{name}' must contain {minimum} to {maximum} points."
        ));
    }
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            vector(&serde_json::json!({"point":row}), "point", &[dimensions])
                .map_err(|message| format!("'{name}[{index}]': {message}"))
        })
        .collect()
}
