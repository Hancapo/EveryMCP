pub const EPSILON: f64 = 1e-12;

pub fn dot(a: &[f64], b: &[f64]) -> Result<f64, String> {
    if a.len() != b.len() {
        return Err("Vectors must have the same dimension.".into());
    }
    Ok(a.iter().zip(b).map(|(x, y)| x * y).sum())
}

pub fn cross(a: &[f64], b: &[f64]) -> Result<Vec<f64>, String> {
    if a.len() != 3 || b.len() != 3 {
        return Err("Cross product requires two 3D vectors.".into());
    }
    Ok(vec![
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ])
}

pub fn subtract(a: &[f64], b: &[f64]) -> Result<Vec<f64>, String> {
    if a.len() != b.len() {
        return Err("Vectors must have the same dimension.".into());
    }
    Ok(a.iter().zip(b).map(|(x, y)| x - y).collect())
}

pub fn add_scaled(a: &[f64], b: &[f64], scale: f64) -> Result<Vec<f64>, String> {
    if a.len() != b.len() {
        return Err("Vectors must have the same dimension.".into());
    }
    Ok(a.iter().zip(b).map(|(x, y)| x + y * scale).collect())
}

fn scaled_norm(a: &[f64]) -> (f64, f64) {
    let scale = a.iter().fold(0.0f64, |acc, value| acc.max(value.abs()));
    if scale == 0.0 {
        return (0.0, 0.0);
    }
    let squares: f64 = a.iter().map(|value| (value / scale).powi(2)).sum();
    (scale, squares.sqrt())
}

pub fn length(a: &[f64]) -> f64 {
    let (scale, normalized_length) = scaled_norm(a);
    scale * normalized_length
}

pub fn normalize(a: &[f64]) -> Result<Vec<f64>, String> {
    let (scale, normalized_length) = scaled_norm(a);
    if scale == 0.0 {
        return Err("Cannot normalize a zero-length vector.".into());
    }
    Ok(a.iter()
        .map(|value| (value / scale) / normalized_length)
        .collect())
}

pub fn transform(matrix: &[f64], vector: &[f64]) -> Result<Vec<f64>, String> {
    if matrix.len() != 16 || vector.len() != 4 {
        return Err("Transform requires a 4x4 matrix and a 4D vector.".into());
    }
    crate::linear::Matrix::new(4, 4, matrix.to_vec())?.matvec(vector)
}

pub fn multiply4(a: &[f64], b: &[f64]) -> Result<Vec<f64>, String> {
    if a.len() != 16 || b.len() != 16 {
        return Err("Both matrices must have 16 entries.".into());
    }
    Ok(crate::linear::Matrix::new(4, 4, a.to_vec())?
        .multiply(&crate::linear::Matrix::new(4, 4, b.to_vec())?)?
        .data)
}

pub fn invert4(matrix: &[f64]) -> Result<Vec<f64>, String> {
    if matrix.len() != 16 {
        return Err("Matrix must have 16 entries.".into());
    }
    Ok(crate::linear::Matrix::new(4, 4, matrix.to_vec())?
        .inverse()?
        .data)
}
