use super::value;
use crate::input;
use serde_json::Value;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "add" => value(input::number(args, "firstNumber")? + input::number(args, "secondNumber")?),
        "subtract" => value(input::number(args, "minuend")? - input::number(args, "subtrahend")?),
        "multiply" => {
            value(input::number(args, "firstNumber")? * input::number(args, "secondNumber")?)
        }
        "division" | "modulo" => {
            let numerator = input::number(args, "numerator")?;
            let denominator = input::number(args, "denominator")?;
            if denominator == 0.0 {
                return Err("Denominator must not be zero.".into());
            }
            value(if name == "division" {
                numerator / denominator
            } else {
                numerator % denominator
            })
        }
        "sum" | "mean" | "median" | "mode" | "min" | "max" => aggregate(name, args),
        _ => unary(name, args),
    }
}

fn aggregate(name: &str, args: &Value) -> Result<Value, String> {
    let mut numbers = input::vector(args, "numbers", &[])?;
    if numbers.is_empty() {
        return Err("'numbers' must not be empty.".into());
    }
    let result = match name {
        "sum" => numbers.iter().sum(),
        "mean" => numbers.iter().sum::<f64>() / numbers.len() as f64,
        "median" => {
            numbers.sort_by(f64::total_cmp);
            let middle = numbers.len() / 2;
            if numbers.len() % 2 == 1 {
                numbers[middle]
            } else {
                numbers[middle - 1] / 2.0 + numbers[middle] / 2.0
            }
        }
        "mode" => {
            let mut counts: Vec<(f64, usize)> = Vec::new();
            for number in numbers {
                if let Some((_, count)) = counts.iter_mut().find(|(key, _)| *key == number) {
                    *count += 1;
                } else {
                    counts.push((number, 1));
                }
            }
            counts
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.total_cmp(&a.0)))
                .unwrap()
                .0
        }
        "min" => numbers.into_iter().reduce(f64::min).unwrap(),
        "max" => numbers.into_iter().reduce(f64::max).unwrap(),
        _ => return Err("Unknown aggregate.".into()),
    };
    value(result)
}

fn unary(name: &str, args: &Value) -> Result<Value, String> {
    let number = input::number(args, "number")?;
    let result = match name {
        "floor" => number.floor(),
        "ceiling" => number.ceil(),
        "round" => (number + 0.5).floor(),
        "sin" => number.sin(),
        "arcsin" => number.asin(),
        "cos" => number.cos(),
        "arccos" => number.acos(),
        "tan" => number.tan(),
        "arctan" => number.atan(),
        "radiansToDegrees" => number * 180.0 / std::f64::consts::PI,
        "degreesToRadians" => number * std::f64::consts::PI / 180.0,
        _ => return Err("Unknown unary operation.".into()),
    };
    value(result)
}
