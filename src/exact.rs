use crate::input;
use num_bigint::BigInt;
use num_traits::{One, Signed, Zero};
use serde_json::Value;

pub fn gcd(mut a: BigInt, mut b: BigInt) -> BigInt {
    while !b.is_zero() {
        let remainder = &a % &b;
        a = b;
        b = remainder;
    }
    a.abs()
}

pub fn extended_gcd(a: BigInt, b: BigInt) -> (BigInt, BigInt, BigInt) {
    let (mut old_r, mut r) = (a, b);
    let (mut old_s, mut s) = (BigInt::one(), BigInt::zero());
    let (mut old_t, mut t) = (BigInt::zero(), BigInt::one());
    while !r.is_zero() {
        let quotient = &old_r / &r;
        (old_r, r) = (r.clone(), old_r - &quotient * &r);
        (old_s, s) = (s.clone(), old_s - &quotient * &s);
        (old_t, t) = (t.clone(), old_t - &quotient * &t);
    }
    if old_r.is_negative() {
        (-old_r, -old_s, -old_t)
    } else {
        (old_r, old_s, old_t)
    }
}

pub fn modulo(value: &BigInt, modulus: &BigInt) -> BigInt {
    ((value % modulus) + modulus) % modulus
}

pub fn integer_power(base: &BigInt, exponent: u32) -> Result<BigInt, String> {
    const MAX_RESULT_DIGITS: usize = 10_000;
    let magnitude = base.abs();
    let base_digits = magnitude.to_str_radix(10).len();
    if magnitude > BigInt::one()
        && base_digits.saturating_mul(exponent as usize) > MAX_RESULT_DIGITS
    {
        return Err("Power result may exceed 10000 decimal digits.".into());
    }
    let result = base.pow(exponent);
    if result.abs().to_str_radix(10).len() > MAX_RESULT_DIGITS {
        return Err("Power result exceeds 10000 decimal digits.".into());
    }
    Ok(result)
}

pub struct Fraction {
    pub numerator: BigInt,
    pub denominator: BigInt,
}

impl Fraction {
    pub fn new(mut numerator: BigInt, mut denominator: BigInt) -> Result<Self, String> {
        if denominator.is_zero() {
            return Err("Fraction denominator must not be zero.".into());
        }
        if denominator.is_negative() {
            numerator = -numerator;
            denominator = -denominator;
        }
        let divisor = gcd(numerator.clone(), denominator.clone());
        Ok(Self {
            numerator: numerator / divisor.clone(),
            denominator: denominator / divisor,
        })
    }

    pub fn parse(value: &Value, name: &str) -> Result<Self, String> {
        let obj = input::property(value, name)?;
        Self::new(
            input::big_integer(obj, "numerator")?,
            input::big_integer(obj, "denominator")?,
        )
    }

    pub fn add(&self, other: &Self) -> Result<Self, String> {
        Self::new(
            &self.numerator * &other.denominator + &other.numerator * &self.denominator,
            &self.denominator * &other.denominator,
        )
    }
    pub fn subtract(&self, other: &Self) -> Result<Self, String> {
        Self::new(
            &self.numerator * &other.denominator - &other.numerator * &self.denominator,
            &self.denominator * &other.denominator,
        )
    }
    pub fn multiply(&self, other: &Self) -> Result<Self, String> {
        Self::new(
            &self.numerator * &other.numerator,
            &self.denominator * &other.denominator,
        )
    }
    pub fn divide(&self, other: &Self) -> Result<Self, String> {
        Self::new(
            &self.numerator * &other.denominator,
            &self.denominator * &other.numerator,
        )
    }
}
