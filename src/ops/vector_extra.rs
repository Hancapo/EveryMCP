use super::{array, value};
use crate::{geometry_math, input, math};
use serde_json::Value;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "vector_n" => vector_n(args),
        "vector_special" => vector_special(args),
        _ => Err(format!("Unknown tool '{name}'.")),
    }
}

fn vector_n(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let a = input::vector(args, "a", &[])?;
    if !(2..=32).contains(&a.len()) {
        return Err("Vector dimension must be 2..32.".into());
    }
    let result = match operation {
        "scale" => {
            let scalar = input::number(args, "scalar")?;
            a.iter().map(|x| x * scalar).collect()
        }
        "clamp" => {
            let lower = input::vector(args, "lower", &[a.len()])?;
            let upper = input::vector(args, "upper", &[a.len()])?;
            if lower.iter().zip(&upper).any(|(low, high)| low > high) {
                return Err("Clamp lower bound exceeds upper bound.".into());
            }
            a.iter()
                .enumerate()
                .map(|(i, x)| x.clamp(lower[i], upper[i]))
                .collect()
        }
        "add" | "subtract" | "hadamard" | "divide" | "min" | "max" | "lerp" | "project"
        | "reject" | "reflect" => {
            let b = input::vector(args, "b", &[a.len()])?;
            match operation {
                "add" => math::add_scaled(&a, &b, 1.0)?,
                "subtract" => math::subtract(&a, &b)?,
                "hadamard" => a.iter().zip(&b).map(|(x, y)| x * y).collect(),
                "divide" => {
                    if b.contains(&0.0) {
                        return Err("Componentwise divisor contains zero.".into());
                    }
                    a.iter().zip(&b).map(|(x, y)| x / y).collect()
                }
                "min" => a.iter().zip(&b).map(|(x, y)| x.min(*y)).collect(),
                "max" => a.iter().zip(&b).map(|(x, y)| x.max(*y)).collect(),
                "lerp" => {
                    math::add_scaled(&a, &math::subtract(&b, &a)?, input::number(args, "t")?)?
                }
                "project" | "reject" => {
                    let unit = math::normalize(&b)?;
                    let coefficient = math::dot(&a, &unit)?;
                    let projection = unit.iter().map(|x| x * coefficient).collect::<Vec<_>>();
                    if operation == "project" {
                        projection
                    } else {
                        math::subtract(&a, &projection)?
                    }
                }
                _ => {
                    let normal = math::normalize(&b)?;
                    math::add_scaled(&a, &normal, -2.0 * math::dot(&a, &normal)?)?
                }
            }
        }
        _ => return Err("Unsupported vector_n operation.".into()),
    };
    array("vector", result)
}

fn vector_special(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    match operation {
        "perp" => {
            let a = input::vector(args, "a", &[2])?;
            array("vector", vec![-a[1], a[0]])
        }
        "signed_angle" => {
            let a = math::normalize(&input::vector(args, "a", &[2])?)?;
            let b = math::normalize(&input::vector(args, "b", &[2])?)?;
            value(geometry_math::cross2(&a, &b).atan2(math::dot(&a, &b)?))
        }
        "scalar_triple" => {
            let a = input::vector(args, "a", &[3])?;
            let b = input::vector(args, "b", &[3])?;
            let c = input::vector(args, "c", &[3])?;
            value(math::dot(&a, &math::cross(&b, &c)?)?)
        }
        "homogenize" => {
            let mut a = input::vector(args, "a", &[3])?;
            a.push(input::optional_number(args, "w")?.unwrap_or(1.0));
            array("vector", a)
        }
        "perspective_divide" => {
            let a = input::vector(args, "a", &[4])?;
            if a[3] == 0.0 {
                return Err("Homogeneous w must be nonzero.".into());
            }
            array("vector", a[..3].iter().map(|x| x / a[3]).collect())
        }
        _ => Err("Unsupported vector_special operation.".into()),
    }
}
