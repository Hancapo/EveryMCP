use crate::input;
use nalgebra::{DMatrix, DVector};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f64>, // Logical row-major order.
}

impl Matrix {
    pub fn new(rows: usize, cols: usize, data: Vec<f64>) -> Result<Self, String> {
        if rows == 0 || cols == 0 || rows > 32 || cols > 32 || data.len() != rows * cols {
            return Err(
                "Matrix shape must be 1..32 by 1..32 with exactly rows*cols values.".into(),
            );
        }
        if data.iter().any(|x| !x.is_finite()) {
            return Err("Matrix entries must be finite.".into());
        }
        Ok(Self { rows, cols, data })
    }

    pub fn parse(args: &Value, name: &str) -> Result<Self, String> {
        let object = input::property(args, name)?;
        let rows = input::int(object, "rows")?;
        let cols = input::int(object, "cols")?;
        if !(1..=32).contains(&rows) || !(1..=32).contains(&cols) {
            return Err("Matrix dimensions must be 1..32.".into());
        }
        let data = input::vector(object, "data", &[(rows * cols) as usize])?;
        Self::new(rows as usize, cols as usize, data)
    }

    pub fn from_na(matrix: &DMatrix<f64>) -> Result<Self, String> {
        let data = (0..matrix.nrows())
            .flat_map(|r| (0..matrix.ncols()).map(move |c| matrix[(r, c)]))
            .collect();
        Self::new(matrix.nrows(), matrix.ncols(), data)
    }

    pub fn to_na(&self) -> DMatrix<f64> {
        DMatrix::from_row_slice(self.rows, self.cols, &self.data)
    }

    pub fn json(&self) -> Value {
        json!({"rows":self.rows,"cols":self.cols,"data":self.data})
    }

    pub fn square(&self) -> Result<(), String> {
        if self.rows != self.cols {
            Err("Operation requires a square matrix.".into())
        } else {
            Ok(())
        }
    }

    pub fn multiply(&self, other: &Self) -> Result<Self, String> {
        if self.cols != other.rows {
            return Err("Matrix dimensions are incompatible for multiplication.".into());
        }
        Self::from_na(&(self.to_na() * other.to_na()))
    }

    pub fn matvec(&self, vector: &[f64]) -> Result<Vec<f64>, String> {
        if vector.len() != self.cols {
            return Err("Vector length must match matrix columns.".into());
        }
        let result = &self.to_na() * DVector::from_row_slice(vector);
        let values = result.iter().copied().collect::<Vec<_>>();
        if values.iter().any(|x| !x.is_finite()) {
            return Err("Result is not finite.".into());
        }
        Ok(values)
    }

    pub fn elementwise(
        &self,
        other: &Self,
        operation: impl Fn(f64, f64) -> f64,
    ) -> Result<Self, String> {
        if self.rows != other.rows || self.cols != other.cols {
            return Err("Matrix shapes must match.".into());
        }
        Self::new(
            self.rows,
            self.cols,
            self.data
                .iter()
                .zip(&other.data)
                .map(|(a, b)| operation(*a, *b))
                .collect(),
        )
    }

    pub fn scale(&self, scalar: f64) -> Result<Self, String> {
        Self::new(
            self.rows,
            self.cols,
            self.data.iter().map(|x| x * scalar).collect(),
        )
    }

    pub fn outer(u: &[f64], v: &[f64]) -> Result<Self, String> {
        Self::new(
            u.len(),
            v.len(),
            u.iter()
                .flat_map(|x| v.iter().map(move |y| x * y))
                .collect(),
        )
    }

    pub fn transpose(&self) -> Result<Self, String> {
        Self::from_na(&self.to_na().transpose())
    }

    pub fn inverse(&self) -> Result<Self, String> {
        self.square()?;
        let inverse = self.to_na().try_inverse().ok_or("Matrix is singular.")?;
        Self::from_na(&inverse)
    }

    pub fn solve(&self, rhs: &[f64]) -> Result<Vec<f64>, String> {
        self.square()?;
        if rhs.len() != self.rows {
            return Err("Right-hand side length must match matrix rows.".into());
        }
        let solution = self
            .to_na()
            .lu()
            .solve(&DVector::from_row_slice(rhs))
            .ok_or("Matrix is singular.")?;
        let values = solution.iter().copied().collect::<Vec<_>>();
        if values.iter().any(|x| !x.is_finite()) {
            return Err("Result is not finite.".into());
        }
        Ok(values)
    }

    pub fn residual(&self, solution: &[f64], rhs: &[f64]) -> Result<f64, String> {
        if solution.len() != self.cols || rhs.len() != self.rows {
            return Err("Residual dimensions are incompatible.".into());
        }
        let mut maximum = 0.0f64;
        for (row, expected) in rhs.iter().enumerate() {
            let actual = (0..self.cols)
                .map(|col| self.data[row * self.cols + col] * solution[col])
                .sum::<f64>();
            maximum = maximum.max((actual - expected).abs());
        }
        if !maximum.is_finite() {
            return Err("Residual is not finite.".into());
        }
        Ok(maximum)
    }
}

pub fn tolerance(
    matrix: &Matrix,
    singular_values: &[f64],
    requested: Option<f64>,
) -> Result<f64, String> {
    if let Some(value) = requested {
        if value < 0.0 || !value.is_finite() {
            return Err("Tolerance must be finite and nonnegative.".into());
        }
        return Ok(value);
    }
    let largest = singular_values.iter().copied().fold(0.0f64, f64::max);
    Ok(f64::EPSILON * (matrix.rows.max(matrix.cols) as f64) * largest)
}
