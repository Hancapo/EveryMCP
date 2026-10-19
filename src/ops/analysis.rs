use super::checked_array;
use crate::input;
use nalgebra::{DMatrix, DVector};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

const MAX_EXPRESSION_CHARS: usize = 10_000;
const MAX_SWEEP: usize = 100_000;
const MAX_SAMPLES: usize = 100_000;
const MAX_DEGREE: usize = 10;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "expression_evaluate" => expression_evaluate(args),
        "curve_fit" => curve_fit(args),
        _ => unreachable!(),
    }
}

#[derive(Debug)]
enum Expr {
    Number(f64),
    Variable(String),
    Unary(char, Box<Expr>),
    Binary(char, Box<Expr>, Box<Expr>),
    Compare(&'static str, Box<Expr>, Box<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

struct ExprParser<'a> {
    text: &'a [u8],
    pos: usize,
}

impl ExprParser<'_> {
    fn error(&self, message: &str) -> String {
        format!("{message} at position {}.", self.pos + 1)
    }

    fn skip(&mut self) {
        while self.pos < self.text.len() && self.text[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.text.get(self.pos).copied()
    }

    fn eat(&mut self, token: &str) -> bool {
        self.skip();
        if self.text[self.pos..].starts_with(token.as_bytes()) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    // conditional := comparison ('?' conditional ':' conditional)?
    fn conditional(&mut self, depth: usize) -> Result<Expr, String> {
        if depth > 200 {
            return Err(self.error("Expression is nested too deeply"));
        }
        let condition = self.comparison(depth)?;
        if self.eat("?") {
            let yes = self.conditional(depth + 1)?;
            if !self.eat(":") {
                return Err(self.error("Expected ':'"));
            }
            let no = self.conditional(depth + 1)?;
            return Ok(Expr::Conditional(
                Box::new(condition),
                Box::new(yes),
                Box::new(no),
            ));
        }
        Ok(condition)
    }

    fn comparison(&mut self, depth: usize) -> Result<Expr, String> {
        let left = self.additive(depth)?;
        for op in ["<=", ">=", "==", "!=", "<", ">"] {
            if self.eat(op) {
                let right = self.additive(depth)?;
                return Ok(Expr::Compare(op, Box::new(left), Box::new(right)));
            }
        }
        Ok(left)
    }

    fn additive(&mut self, depth: usize) -> Result<Expr, String> {
        let mut left = self.multiplicative(depth)?;
        loop {
            let op = match self.peek() {
                Some(c @ (b'+' | b'-')) => c as char,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.multiplicative(depth)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn multiplicative(&mut self, depth: usize) -> Result<Expr, String> {
        let mut left = self.unary(depth)?;
        loop {
            self.skip();
            if self.text[self.pos..].starts_with(b"**") {
                return Ok(left);
            }
            let op = match self.peek() {
                Some(c @ (b'*' | b'/' | b'%')) => c as char,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.unary(depth)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    // Unary minus binds looser than '^', so -2^2 is -4.
    fn unary(&mut self, depth: usize) -> Result<Expr, String> {
        if depth > 200 {
            return Err(self.error("Expression is nested too deeply"));
        }
        match self.peek() {
            Some(b'-') => {
                self.pos += 1;
                Ok(Expr::Unary('-', Box::new(self.unary(depth + 1)?)))
            }
            Some(b'+') => {
                self.pos += 1;
                self.unary(depth + 1)
            }
            Some(b'!') if self.text.get(self.pos + 1) != Some(&b'=') => {
                self.pos += 1;
                Ok(Expr::Unary('!', Box::new(self.unary(depth + 1)?)))
            }
            _ => self.power(depth),
        }
    }

    fn power(&mut self, depth: usize) -> Result<Expr, String> {
        let base = self.primary(depth)?;
        if self.eat("^") || self.eat("**") {
            let exponent = self.unary(depth + 1)?;
            return Ok(Expr::Binary('^', Box::new(base), Box::new(exponent)));
        }
        Ok(base)
    }

    fn primary(&mut self, depth: usize) -> Result<Expr, String> {
        match self.peek() {
            Some(b'(') => {
                self.pos += 1;
                let inner = self.conditional(depth + 1)?;
                if !self.eat(")") {
                    return Err(self.error("Expected ')'"));
                }
                Ok(inner)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.pos;
                while self.pos < self.text.len()
                    && (self.text[self.pos].is_ascii_alphanumeric() || self.text[self.pos] == b'.')
                {
                    // Allow exponents such as 1e-3.
                    if matches!(self.text[self.pos], b'e' | b'E')
                        && matches!(self.text.get(self.pos + 1), Some(b'+' | b'-'))
                        && !self.text[start..self.pos].starts_with(b"0x")
                    {
                        self.pos += 1;
                    }
                    self.pos += 1;
                }
                let raw = std::str::from_utf8(&self.text[start..self.pos]).unwrap();
                let value =
                    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
                        u64::from_str_radix(hex, 16).map(|v| v as f64).ok()
                    } else if let Some(binary) = raw.strip_prefix("0b") {
                        u64::from_str_radix(binary, 2).map(|v| v as f64).ok()
                    } else {
                        raw.parse::<f64>().ok()
                    };
                value
                    .map(Expr::Number)
                    .ok_or_else(|| format!("Invalid number '{raw}' at position {}.", start + 1))
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let start = self.pos;
                while self.pos < self.text.len()
                    && (self.text[self.pos].is_ascii_alphanumeric() || self.text[self.pos] == b'_')
                {
                    self.pos += 1;
                }
                let name = std::str::from_utf8(&self.text[start..self.pos])
                    .unwrap()
                    .to_owned();
                if self.eat("(") {
                    let mut arguments = Vec::new();
                    if !self.eat(")") {
                        loop {
                            arguments.push(self.conditional(depth + 1)?);
                            if self.eat(")") {
                                break;
                            }
                            if !self.eat(",") {
                                return Err(self.error("Expected ',' or ')'"));
                            }
                        }
                    }
                    return Ok(Expr::Call(name, arguments));
                }
                Ok(Expr::Variable(name))
            }
            Some(c) => Err(self.error(&format!("Unexpected '{}'", c as char))),
            None => Err(self.error("Unexpected end of expression")),
        }
    }
}

struct Context<'a> {
    variables: &'a HashMap<String, f64>,
    degrees: bool,
}

fn constant(name: &str) -> Option<f64> {
    Some(match name {
        "pi" | "PI" => std::f64::consts::PI,
        "tau" | "TAU" => std::f64::consts::TAU,
        "e" | "E" => std::f64::consts::E,
        "inf" | "infinity" => f64::INFINITY,
        _ => return None,
    })
}

fn call(name: &str, a: &[f64], context: &Context) -> Result<f64, String> {
    let to_rad = |x: f64| if context.degrees { x.to_radians() } else { x };
    let from_rad = |x: f64| if context.degrees { x.to_degrees() } else { x };
    let arity = |n: usize| -> Result<(), String> {
        if a.len() == n {
            Ok(())
        } else {
            Err(format!(
                "{name}() takes {n} argument{}.",
                if n == 1 { "" } else { "s" }
            ))
        }
    };
    let one = |f: fn(f64) -> f64| -> Result<f64, String> {
        arity(1)?;
        Ok(f(a[0]))
    };
    match name {
        "sin" => {
            arity(1)?;
            Ok(to_rad(a[0]).sin())
        }
        "cos" => {
            arity(1)?;
            Ok(to_rad(a[0]).cos())
        }
        "tan" => {
            arity(1)?;
            Ok(to_rad(a[0]).tan())
        }
        "asin" => {
            arity(1)?;
            Ok(from_rad(a[0].asin()))
        }
        "acos" => {
            arity(1)?;
            Ok(from_rad(a[0].acos()))
        }
        "atan" => {
            arity(1)?;
            Ok(from_rad(a[0].atan()))
        }
        "atan2" => {
            arity(2)?;
            Ok(from_rad(a[0].atan2(a[1])))
        }
        "sinh" => one(f64::sinh),
        "cosh" => one(f64::cosh),
        "tanh" => one(f64::tanh),
        "sqrt" => one(f64::sqrt),
        "cbrt" => one(f64::cbrt),
        "abs" => one(f64::abs),
        "exp" => one(f64::exp),
        "ln" => one(f64::ln),
        "log10" => one(f64::log10),
        "log2" => one(f64::log2),
        "log" => match a.len() {
            1 => Ok(a[0].ln()),
            2 => Ok(a[0].log(a[1])),
            _ => Err("log() takes 1 or 2 arguments (value, base).".into()),
        },
        "floor" => one(f64::floor),
        "ceil" => one(f64::ceil),
        "round" => one(f64::round),
        "trunc" => one(f64::trunc),
        "sign" => one(|x| if x == 0.0 { 0.0 } else { x.signum() }),
        "fract" => one(f64::fract),
        "pow" => {
            arity(2)?;
            Ok(a[0].powf(a[1]))
        }
        "hypot" => match a.len() {
            0 => Err("hypot() needs at least one argument.".into()),
            _ => Ok(a.iter().map(|x| x * x).sum::<f64>().sqrt()),
        },
        "min" | "max" if !a.is_empty() => Ok(a.iter().copied().fold(
            if name == "min" {
                f64::INFINITY
            } else {
                f64::NEG_INFINITY
            },
            if name == "min" { f64::min } else { f64::max },
        )),
        "sum" => Ok(a.iter().sum()),
        "mean" if !a.is_empty() => Ok(a.iter().sum::<f64>() / a.len() as f64),
        "clamp" => {
            arity(3)?;
            Ok(a[0].clamp(a[1].min(a[2]), a[1].max(a[2])))
        }
        "lerp" => {
            arity(3)?;
            Ok(a[0] + (a[1] - a[0]) * a[2])
        }
        "deg" => {
            arity(1)?;
            Ok(a[0].to_degrees())
        }
        "rad" => {
            arity(1)?;
            Ok(a[0].to_radians())
        }
        "mod" => {
            arity(2)?;
            Ok(a[0].rem_euclid(a[1]))
        }
        _ => Err(format!("Unknown function '{name}'.")),
    }
}

fn evaluate(expr: &Expr, context: &Context) -> Result<f64, String> {
    Ok(match expr {
        Expr::Number(n) => *n,
        Expr::Variable(name) => match context.variables.get(name) {
            Some(&v) => v,
            None => constant(name).ok_or_else(|| format!("Unknown variable '{name}'."))?,
        },
        Expr::Unary('-', inner) => -evaluate(inner, context)?,
        Expr::Unary(_, inner) => f64::from(evaluate(inner, context)? == 0.0),
        Expr::Binary(op, left, right) => {
            let (a, b) = (evaluate(left, context)?, evaluate(right, context)?);
            match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' => a / b,
                '%' => a % b,
                _ => a.powf(b),
            }
        }
        Expr::Compare(op, left, right) => {
            let (a, b) = (evaluate(left, context)?, evaluate(right, context)?);
            f64::from(match *op {
                "<" => a < b,
                "<=" => a <= b,
                ">" => a > b,
                ">=" => a >= b,
                "==" => a == b,
                _ => a != b,
            })
        }
        Expr::Conditional(condition, yes, no) => {
            if evaluate(condition, context)? != 0.0 {
                evaluate(yes, context)?
            } else {
                evaluate(no, context)?
            }
        }
        Expr::Call(name, arguments) => {
            let values = arguments
                .iter()
                .map(|a| evaluate(a, context))
                .collect::<Result<Vec<_>, _>>()?;
            call(name, &values, context)?
        }
    })
}

fn finite(value: f64) -> Result<Value, String> {
    if value.is_finite() {
        Ok(json!(value))
    } else if value.is_nan() {
        Err("The expression is undefined here (NaN).".into())
    } else {
        Err("The expression overflows to infinity.".into())
    }
}

fn expression_evaluate(args: &Value) -> Result<Value, String> {
    let text = input::string(args, "expression")?;
    if text.len() > MAX_EXPRESSION_CHARS {
        return Err("'expression' may be at most 10,000 characters.".into());
    }
    let mut parser = ExprParser {
        text: text.as_bytes(),
        pos: 0,
    };
    let expr = parser.conditional(0)?;
    if parser.peek().is_some() {
        return Err(parser.error("Unexpected trailing input"));
    }
    let mut variables = HashMap::new();
    if let Some(raw) = args.get("variables") {
        let object = raw
            .as_object()
            .ok_or("'variables' must be an object of numbers.")?;
        for (name, value) in object {
            let number = value
                .as_f64()
                .ok_or_else(|| format!("Variable '{name}' must be a number."))?;
            variables.insert(name.clone(), number);
        }
    }
    let degrees = match args
        .get("angleUnit")
        .and_then(Value::as_str)
        .unwrap_or("radians")
    {
        "radians" => false,
        "degrees" => true,
        _ => return Err("'angleUnit' must be radians or degrees.".into()),
    };
    match args.get("sweepVariable").and_then(Value::as_str) {
        None => {
            let context = Context {
                variables: &variables,
                degrees,
            };
            Ok(json!({"value":finite(evaluate(&expr, &context)?)?}))
        }
        Some(name) => {
            let values = input::vector(args, "sweepValues", &[])?;
            if values.is_empty() || values.len() > MAX_SWEEP {
                return Err("'sweepValues' must contain 1 to 100,000 numbers.".into());
            }
            let mut results = Vec::with_capacity(values.len());
            for &x in &values {
                variables.insert(name.to_owned(), x);
                let context = Context {
                    variables: &variables,
                    degrees,
                };
                let y = evaluate(&expr, &context)?;
                results.push(if y.is_finite() { json!(y) } else { Value::Null });
            }
            Ok(json!({"variable":name,"inputs":values,"values":results}))
        }
    }
}

struct Fit {
    coefficients: Vec<f64>,
    predict: Box<dyn Fn(f64) -> f64>,
    equation: String,
    method: &'static str,
}

fn weighted_least_squares(
    design: DMatrix<f64>,
    y: &[f64],
    weights: &[f64],
) -> Result<Vec<f64>, String> {
    let mut a = design;
    let mut b = DVector::from_column_slice(y);
    for (row, &w) in weights.iter().enumerate() {
        let s = w.sqrt();
        a.row_mut(row).scale_mut(s);
        b[row] *= s;
    }
    let svd = a.svd(true, true);
    let max_singular = svd.singular_values.max();
    if max_singular == 0.0 {
        return Err("The data cannot determine the model (all-zero design).".into());
    }
    let solution = svd
        .solve(&b, max_singular * 1e-12)
        .map_err(|e| format!("Least squares failed: {e}"))?;
    Ok(solution.iter().copied().collect())
}

/// Levenberg-Marquardt for y = a·g(b, x), started from a log-space estimate.
fn refine(x: &[f64], y: &[f64], w: &[f64], mut a: f64, mut b: f64, kind: &str) -> (f64, f64) {
    let model = |a: f64, b: f64, x: f64| {
        if kind == "exponential" {
            a * (b * x).exp()
        } else {
            a * x.powf(b)
        }
    };
    let cost = |a: f64, b: f64| -> f64 {
        x.iter()
            .zip(y)
            .zip(w)
            .map(|((&x, &y), &w)| w * (y - model(a, b, x)).powi(2))
            .sum()
    };
    let mut lambda = 1e-3;
    let mut current = cost(a, b);
    for _ in 0..200 {
        let (mut jtj, mut jtr) = ([[0.0; 2]; 2], [0.0; 2]);
        for ((&x, &y), &w) in x.iter().zip(y).zip(w) {
            let g = if kind == "exponential" {
                (b * x).exp()
            } else {
                x.powf(b)
            };
            let dg = if kind == "exponential" {
                x * g
            } else {
                x.ln() * g
            };
            let j = [g, a * dg];
            let r = y - a * g;
            for p in 0..2 {
                jtr[p] += w * j[p] * r;
                for q in 0..2 {
                    jtj[p][q] += w * j[p] * j[q];
                }
            }
        }
        let m = [
            [jtj[0][0] * (1.0 + lambda), jtj[0][1]],
            [jtj[1][0], jtj[1][1] * (1.0 + lambda)],
        ];
        let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
        if det.abs() < 1e-300 {
            break;
        }
        let da = (jtr[0] * m[1][1] - jtr[1] * m[0][1]) / det;
        let db = (m[0][0] * jtr[1] - m[1][0] * jtr[0]) / det;
        let trial = cost(a + da, b + db);
        if trial.is_finite() && trial < current {
            let improvement = current - trial;
            a += da;
            b += db;
            current = trial;
            lambda = (lambda / 10.0).max(1e-12);
            if improvement <= current * 1e-15
                || (da.abs() <= 1e-15 * a.abs() && db.abs() <= 1e-15 * b.abs().max(1e-300))
            {
                break;
            }
        } else {
            lambda *= 10.0;
            if lambda > 1e12 {
                break;
            }
        }
    }
    (a, b)
}

fn format_number(v: f64) -> String {
    let text = format!("{v:.6e}");
    match text.split_once('e') {
        Some((mantissa, "0")) => mantissa
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned(),
        _ if (1e-4..1e6).contains(&v.abs()) => format!("{v}"),
        _ => text,
    }
}

fn curve_fit(args: &Value) -> Result<Value, String> {
    let x = input::vector(args, "x", &[])?;
    let y = input::vector(args, "y", &[x.len()])?;
    if x.len() < 2 || x.len() > MAX_SAMPLES {
        return Err("'x' and 'y' must contain 2 to 100,000 samples.".into());
    }
    let weights = match args.get("weights") {
        Some(_) => {
            let w = input::vector(args, "weights", &[x.len()])?;
            if w.iter().any(|&w| !(w >= 0.0 && w.is_finite())) || w.iter().all(|&w| w == 0.0) {
                return Err(
                    "'weights' must be nonnegative with at least one positive value.".into(),
                );
            }
            w
        }
        None => vec![1.0; x.len()],
    };
    let model = input::string(args, "model")?;
    let fit = match model {
        "linear" | "polynomial" => {
            let degree = if model == "linear" {
                1
            } else {
                input::optional_int(args, "degree")?.unwrap_or(2) as usize
            };
            if !(1..=MAX_DEGREE).contains(&degree) {
                return Err("'degree' must be between 1 and 10.".into());
            }
            if x.len() <= degree {
                return Err(format!(
                    "A degree-{degree} fit needs at least {} samples.",
                    degree + 1
                ));
            }
            // Fit on centred, scaled x for conditioning, then expand back to powers of x.
            let center = x.iter().sum::<f64>() / x.len() as f64;
            let spread = x
                .iter()
                .map(|v| (v - center).abs())
                .fold(0.0, f64::max)
                .max(1e-300);
            let design = DMatrix::from_fn(x.len(), degree + 1, |r, c| {
                ((x[r] - center) / spread).powi(c as i32)
            });
            let scaled = weighted_least_squares(design, &y, &weights)?;
            let mut coefficients = vec![0.0; degree + 1];
            for (k, &ck) in scaled.iter().enumerate() {
                // ck * ((x - center) / spread)^k expanded with the binomial theorem.
                for (j, coefficient) in coefficients.iter_mut().enumerate().take(k + 1) {
                    let binomial = (0..j).fold(1.0, |acc, i| acc * (k - i) as f64 / (i + 1) as f64);
                    *coefficient +=
                        ck * binomial * (-center).powi((k - j) as i32) / spread.powi(k as i32);
                }
            }
            let terms: Vec<String> = coefficients
                .iter()
                .enumerate()
                .map(|(power, c)| match power {
                    0 => format_number(*c),
                    1 => format!("{}·x", format_number(*c)),
                    _ => format!("{}·x^{power}", format_number(*c)),
                })
                .collect();
            let predict_scaled = scaled.clone();
            Fit {
                equation: format!("y = {}", terms.join(" + ")),
                coefficients,
                predict: Box::new(move |v| {
                    let t = (v - center) / spread;
                    predict_scaled.iter().rev().fold(0.0, |acc, c| acc * t + c)
                }),
                method: "least_squares_svd",
            }
        }
        "logarithmic" => {
            if x.iter().any(|&v| v <= 0.0) {
                return Err("A logarithmic fit needs every x > 0.".into());
            }
            let design = DMatrix::from_fn(x.len(), 2, |r, c| if c == 0 { 1.0 } else { x[r].ln() });
            let c = weighted_least_squares(design, &y, &weights)?;
            let (a, b) = (c[0], c[1]);
            Fit {
                equation: format!("y = {} + {}·ln(x)", format_number(a), format_number(b)),
                coefficients: vec![a, b],
                predict: Box::new(move |v| a + b * v.ln()),
                method: "least_squares_svd",
            }
        }
        "exponential" | "power" => {
            let exponential = model == "exponential";
            if y.iter().any(|&v| v <= 0.0) || (!exponential && x.iter().any(|&v| v <= 0.0)) {
                return Err(if exponential {
                    "An exponential fit needs every y > 0.".into()
                } else {
                    "A power fit needs every x > 0 and y > 0.".into()
                });
            }
            let design = DMatrix::from_fn(x.len(), 2, |r, c| match c {
                0 => 1.0,
                _ if exponential => x[r],
                _ => x[r].ln(),
            });
            let ln_y: Vec<f64> = y.iter().map(|v| v.ln()).collect();
            let start = weighted_least_squares(design, &ln_y, &weights)?;
            let (a, b) = refine(&x, &y, &weights, start[0].exp(), start[1], model);
            Fit {
                equation: if exponential {
                    format!("y = {}·e^({}·x)", format_number(a), format_number(b))
                } else {
                    format!("y = {}·x^{}", format_number(a), format_number(b))
                },
                coefficients: vec![a, b],
                predict: Box::new(move |v| {
                    if exponential {
                        a * (b * v).exp()
                    } else {
                        a * v.powf(b)
                    }
                }),
                method: "levenberg_marquardt",
            }
        }
        _ => {
            return Err(
                "'model' must be linear, polynomial, exponential, power or logarithmic.".into(),
            );
        }
    };
    let total_weight: f64 = weights.iter().sum();
    let mean = x
        .iter()
        .zip(&y)
        .zip(&weights)
        .map(|((_, y), w)| w * y)
        .sum::<f64>()
        / total_weight;
    let (mut ss_res, mut ss_tot, mut max_residual) = (0.0, 0.0, 0.0f64);
    for ((&xi, &yi), &w) in x.iter().zip(&y).zip(&weights) {
        let residual = yi - (fit.predict)(xi);
        ss_res += w * residual * residual;
        ss_tot += w * (yi - mean).powi(2);
        if w > 0.0 {
            max_residual = max_residual.max(residual.abs());
        }
    }
    let r2 = if ss_tot > 0.0 {
        1.0 - ss_res / ss_tot
    } else if ss_res == 0.0 {
        1.0
    } else {
        0.0
    };
    let names: &[&str] = match model {
        "linear" | "logarithmic" => &["a", "b"],
        "exponential" | "power" => &["a", "b"],
        _ => &[],
    };
    let mut named = Map::new();
    for (name, value) in names.iter().zip(&fit.coefficients) {
        named.insert((*name).to_owned(), json!(value));
    }
    let mut result = json!({
        "model":model,"method":fit.method,"equation":fit.equation,
        "coefficients":checked_array(fit.coefficients.clone())?,
        "r2":r2,"rmse":(ss_res / total_weight).sqrt(),"maxAbsResidual":max_residual,"samples":x.len()
    });
    if model == "polynomial" || model == "linear" {
        result["coefficientOrder"] = json!("ascending powers of x (c0 + c1·x + ...)");
    }
    if !named.is_empty() {
        result["parameters"] = Value::Object(named);
    }
    if args.get("predict").is_some() {
        let at = input::vector(args, "predict", &[])?;
        if at.len() > MAX_SAMPLES {
            return Err("'predict' may contain at most 100,000 values.".into());
        }
        result["predictions"] = json!(
            at.iter()
                .map(|&v| {
                    let p = (fit.predict)(v);
                    if p.is_finite() { json!(p) } else { Value::Null }
                })
                .collect::<Vec<_>>()
        );
    }
    Ok(result)
}
