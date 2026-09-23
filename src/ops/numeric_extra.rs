use super::{checked_array, value};
use crate::{exact, input};
use num_bigint::BigInt;
use num_traits::{One, Signed, Zero};
use serde_json::{Value, json};

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "power_root_log" => power_root_log(args),
        "exact_fraction" => exact_fraction(args),
        "gcd_extended" => gcd_extended(args),
        "modular_arithmetic" => modular_arithmetic(args),
        "combinatorics_exact" => combinatorics_exact(args),
        "complex_calculate" => complex_calculate(args),
        "polynomial_evaluate" => polynomial_evaluate(args),
        "linear_system_solve" => linear_system_solve(args),
        "descriptive_statistics" => descriptive_statistics(args),
        "interpolate_samples" => interpolate_samples(args),
        _ => Err(format!("Unknown tool '{name}'.")),
    }
}

fn power_root_log(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let a = input::number(args, "a")?;
    let result = match operation {
        "pow" => a.powf(input::number(args, "b")?),
        "root" => {
            let degree = input::number(args, "b")?;
            if degree == 0.0 {
                return Err("Root degree must be nonzero.".into());
            }
            if a < 0.0 {
                if degree.fract() != 0.0
                    || degree.abs() > i64::MAX as f64
                    || (degree as i64) % 2 == 0
                {
                    return Err("Negative radicand requires an odd integer degree.".into());
                }
                -(-a).powf(1.0 / degree)
            } else {
                a.powf(1.0 / degree)
            }
        }
        "log" => {
            if a <= 0.0 {
                return Err("Logarithm argument must be positive.".into());
            }
            if let Some(base) = input::optional_number(args, "b")? {
                if base <= 0.0 || base == 1.0 {
                    return Err("Logarithm base must be positive and not one.".into());
                }
                a.log(base)
            } else {
                a.ln()
            }
        }
        "exp" => a.exp(),
        _ => return Err("Unsupported power/root/log operation.".into()),
    };
    value(result)
}

fn exact_fraction(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let a = exact::Fraction::parse(args, "a")?;
    if operation == "normalize" {
        return Ok(
            json!({"numerator":a.numerator.to_string(),"denominator":a.denominator.to_string()}),
        );
    }
    let b = exact::Fraction::parse(args, "b")?;
    if operation == "compare" {
        let lhs = &a.numerator * &b.denominator;
        let rhs = &b.numerator * &a.denominator;
        return Ok(json!({"comparison": if lhs < rhs {-1} else if lhs > rhs {1} else {0}}));
    }
    let result = match operation {
        "add" => a.add(&b)?,
        "subtract" => a.subtract(&b)?,
        "multiply" => a.multiply(&b)?,
        "divide" => a.divide(&b)?,
        _ => return Err("Unsupported fraction operation.".into()),
    };
    Ok(
        json!({"numerator":result.numerator.to_string(),"denominator":result.denominator.to_string()}),
    )
}

fn gcd_extended(args: &Value) -> Result<Value, String> {
    let a = input::big_integer(args, "a")?;
    let b = input::big_integer(args, "b")?;
    let (gcd, x, y) = exact::extended_gcd(a.clone(), b.clone());
    let lcm = if gcd.is_zero() {
        BigInt::zero()
    } else {
        (a / gcd.clone()) * b
    };
    Ok(
        json!({"gcd":gcd.to_string(),"lcm":lcm.abs().to_string(),"x":x.to_string(),"y":y.to_string()}),
    )
}

fn modular_arithmetic(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let a = input::big_integer(args, "a")?;
    let modulus = input::big_integer(args, "modulus")?;
    if modulus <= BigInt::one() {
        return Err("Modulus must be greater than one.".into());
    }
    let result = match operation {
        "add" => exact::modulo(&(a + input::big_integer(args, "b")?), &modulus),
        "multiply" => exact::modulo(&(a * input::big_integer(args, "b")?), &modulus),
        "pow" => {
            let exponent = input::big_integer(args, "b")?;
            if exponent < BigInt::zero() {
                return Err("Modular exponent must be nonnegative.".into());
            }
            exact::modulo(&a, &modulus).modpow(&exponent, &modulus)
        }
        "inverse" => {
            let (gcd, x, _) = exact::extended_gcd(a, modulus.clone());
            if gcd != BigInt::one() {
                return Err("Modular inverse does not exist.".into());
            }
            exact::modulo(&x, &modulus)
        }
        _ => return Err("Unsupported modular operation.".into()),
    };
    Ok(json!({"value":result.to_string()}))
}

fn combinatorics_exact(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let n = input::int(args, "n")?;
    if !(0..=5000).contains(&n) {
        return Err("n must be between 0 and 5000.".into());
    }
    let result = match operation {
        "factorial" => (1..=n).fold(BigInt::one(), |acc, k| acc * k),
        "permutation" | "combination" => {
            let k = input::int(args, "k")?;
            if k < 0 || k > n {
                return Err("k must be between 0 and n.".into());
            }
            if operation == "permutation" {
                ((n - k + 1)..=n).fold(BigInt::one(), |acc, x| acc * x)
            } else {
                let k = k.min(n - k);
                let mut result = BigInt::one();
                for i in 1..=k {
                    result = result * (n - k + i) / i;
                }
                result
            }
        }
        _ => return Err("Unsupported combinatorics operation.".into()),
    };
    Ok(json!({"value":result.to_string()}))
}

fn complex_calculate(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let a = input::vector(args, "a", &[2])?;
    if operation == "magnitude" {
        return value(a[0].hypot(a[1]));
    }
    if operation == "argument" {
        return value(a[1].atan2(a[0]));
    }
    let (real, imag) = if operation == "from_polar" {
        if a[0] < 0.0 {
            return Err("Polar radius must be nonnegative.".into());
        }
        (a[0] * a[1].cos(), a[0] * a[1].sin())
    } else {
        let b = input::vector(args, "b", &[2])?;
        match operation {
            "add" => (a[0] + b[0], a[1] + b[1]),
            "subtract" => (a[0] - b[0], a[1] - b[1]),
            "multiply" => (a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]),
            "divide" => {
                let scale = b[0].abs().max(b[1].abs());
                if scale == 0.0 {
                    return Err("Complex divisor must be nonzero.".into());
                }
                let br = b[0] / scale;
                let bi = b[1] / scale;
                let denom = br * br + bi * bi;
                (
                    ((a[0] / scale) * br + (a[1] / scale) * bi) / denom,
                    ((a[1] / scale) * br - (a[0] / scale) * bi) / denom,
                )
            }
            _ => return Err("Unsupported complex operation.".into()),
        }
    };
    if !real.is_finite() || !imag.is_finite() {
        return Err("Result is not finite.".into());
    }
    Ok(json!({"real":real,"imag":imag}))
}

fn polynomial_evaluate(args: &Value) -> Result<Value, String> {
    let coefficients = input::vector(args, "coefficients", &[])?;
    if coefficients.is_empty() || coefficients.len() > 1024 {
        return Err("Polynomial needs 1 to 1024 coefficients.".into());
    }
    let x = input::number(args, "x")?;
    let mut result = coefficients[0];
    let mut derivative = 0.0;
    for &coefficient in &coefficients[1..] {
        derivative = derivative * x + result;
        result = result * x + coefficient;
    }
    if !result.is_finite() || !derivative.is_finite() {
        return Err("Result is not finite.".into());
    }
    Ok(json!({"value":result,"derivative":derivative}))
}

fn linear_system_solve(args: &Value) -> Result<Value, String> {
    let original = input::matrix(args, "matrix", 32)?;
    let n = original.len();
    let rhs = input::vector(args, "b", &[n])?;
    let matrix = crate::linear::Matrix::new(n, n, original.into_iter().flatten().collect())?;
    let solution = matrix.solve(&rhs)?;
    let residual = matrix.residual(&solution, &rhs)?;
    Ok(json!({"solution":checked_array(solution)?,"maxResidual":residual}))
}

fn descriptive_statistics(args: &Value) -> Result<Value, String> {
    let values = input::vector(args, "numbers", &[])?;
    if values.is_empty() {
        return Err("'numbers' must not be empty.".into());
    }
    let sample = input::optional_bool(args, "sample")?;
    if sample && values.len() < 2 {
        return Err("Sample variance needs at least two values.".into());
    }
    let mut mean = 0.0;
    let mut m2 = 0.0;
    for (index, number) in values.iter().enumerate() {
        let delta = number - mean;
        mean += delta / (index + 1) as f64;
        m2 += delta * (number - mean);
    }
    let variance = m2 / (values.len() - usize::from(sample)) as f64;
    let mut sorted = values.clone();
    sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        let position = (sorted.len() - 1) as f64 * p;
        let lower = position.floor() as usize;
        let upper = position.ceil() as usize;
        sorted[lower] + (sorted[upper] - sorted[lower]) * (position - lower as f64)
    };
    let correlation = if args.get("other").is_some() {
        let other = input::vector(args, "other", &[values.len()])?;
        let other_mean = other.iter().sum::<f64>() / other.len() as f64;
        let covariance: f64 = values
            .iter()
            .zip(&other)
            .map(|(x, y)| (x - mean) * (y - other_mean))
            .sum();
        let other_squared: f64 = other.iter().map(|y| (y - other_mean).powi(2)).sum();
        let denominator = (m2 * other_squared).sqrt();
        if denominator == 0.0 {
            return Err("Correlation needs variation in both arrays.".into());
        }
        Some((covariance / denominator).clamp(-1.0, 1.0))
    } else {
        None
    };
    if !mean.is_finite() || !variance.is_finite() || !variance.sqrt().is_finite() {
        return Err("Result is not finite.".into());
    }
    Ok(
        json!({"count":values.len(),"mean":mean,"variance":variance,"standardDeviation":variance.sqrt(),
        "min":sorted[0],"max":sorted[sorted.len()-1],"median":percentile(0.5),"q1":percentile(0.25),"q3":percentile(0.75),
        "correlation":correlation}),
    )
}

fn interpolate_samples(args: &Value) -> Result<Value, String> {
    let mode = input::string(args, "mode")?;
    let p1 = input::number(args, "p1")?;
    let p2 = input::number(args, "p2")?;
    let t = input::number(args, "t")?;
    if !(0.0..=1.0).contains(&t) {
        return Err("t must be in [0,1].".into());
    }
    let result = match mode {
        "linear" => p1 + (p2 - p1) * t,
        "hermite" => {
            let m1 = input::number(args, "tangent1")?;
            let m2 = input::number(args, "tangent2")?;
            let t2 = t * t;
            let t3 = t2 * t;
            (2.0 * t3 - 3.0 * t2 + 1.0) * p1
                + (t3 - 2.0 * t2 + t) * m1
                + (-2.0 * t3 + 3.0 * t2) * p2
                + (t3 - t2) * m2
        }
        "catmull_rom" => {
            let p0 = input::number(args, "p0")?;
            let p3 = input::number(args, "p3")?;
            let t2 = t * t;
            let t3 = t2 * t;
            0.5 * (2.0 * p1
                + (-p0 + p2) * t
                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
        }
        _ => return Err("Unsupported interpolation mode.".into()),
    };
    value(result)
}
