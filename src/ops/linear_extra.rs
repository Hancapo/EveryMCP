use super::{checked_array, value};
use crate::{
    input,
    linear::{self, Matrix},
    math,
};
use nalgebra::{DMatrix, DVector};
use serde_json::{Value, json};

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "matrix_n" => matrix_n(args),
        "matrix_properties" => matrix_properties(args),
        "matrix_factor" => matrix_factor(args),
        "affine_transform" => affine_transform(args),
        "matrix_layout_convert" => matrix_layout_convert(args),
        _ => Err(format!("Unknown tool '{name}'.")),
    }
}

fn matrix_n(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    if operation == "outer" {
        let u = input::vector(args, "u", &[])?;
        let v = input::vector(args, "v", &[])?;
        return Ok(json!({"matrix":Matrix::outer(&u,&v)?.json()}));
    }
    let a = Matrix::parse(args, "a")?;
    match operation {
        "add" | "subtract" | "hadamard" => {
            let b = Matrix::parse(args, "b")?;
            let result = match operation {
                "add" => a.elementwise(&b, |x, y| x + y)?,
                "subtract" => a.elementwise(&b, |x, y| x - y)?,
                _ => a.elementwise(&b, |x, y| x * y)?,
            };
            Ok(json!({"matrix":result.json()}))
        }
        "scale" => Ok(json!({"matrix":a.scale(input::number(args,"scalar")?)?.json()})),
        "transpose" => Ok(json!({"matrix":a.transpose()?.json()})),
        "multiply" => Ok(json!({"matrix":a.multiply(&Matrix::parse(args,"b")?)?.json()})),
        "matvec" => Ok(
            json!({"vector":checked_array(a.matvec(&input::vector(args,"vector",&[a.cols])?)?)?}),
        ),
        _ => Err("Unsupported matrix_n operation.".into()),
    }
}

fn matrix_properties(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let matrix = Matrix::parse(args, "matrix")?;
    matrix.square()?;
    match operation {
        "trace" => value(
            (0..matrix.rows)
                .map(|i| matrix.data[i * matrix.cols + i])
                .sum(),
        ),
        "determinant" => value(matrix.to_na().determinant()),
        "frobenius_norm" => value(math::length(&matrix.data)),
        "infinity_norm" => value(
            (0..matrix.rows)
                .map(|r| {
                    matrix.data[r * matrix.cols..(r + 1) * matrix.cols]
                        .iter()
                        .map(|x| x.abs())
                        .sum::<f64>()
                })
                .fold(0.0, f64::max),
        ),
        "inverse" => Ok(json!({"matrix":matrix.inverse()?.json()})),
        "rank" | "condition" => {
            let singular = matrix.to_na().svd(false, false).singular_values;
            let values = singular.as_slice();
            let threshold =
                linear::tolerance(&matrix, values, input::optional_number(args, "tolerance")?)?;
            let rank = values.iter().filter(|value| **value > threshold).count();
            if operation == "rank" {
                Ok(json!({"rank":rank,"tolerance":threshold}))
            } else if rank < matrix.rows {
                Ok(json!({"value":"Infinity","singular":true,"tolerance":threshold}))
            } else {
                let maximum = values.iter().copied().fold(0.0, f64::max);
                let minimum = values.iter().copied().fold(f64::INFINITY, f64::min);
                let condition = maximum / minimum;
                if !condition.is_finite() {
                    Ok(json!({"value":"Infinity","singular":true,"tolerance":threshold}))
                } else {
                    Ok(json!({"value":condition,"singular":false,"tolerance":threshold}))
                }
            }
        }
        _ => Err("Unsupported matrix property.".into()),
    }
}

fn matrix_factor(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let matrix = Matrix::parse(args, "matrix")?;
    match operation {
        "lu" => {
            let lu = matrix.to_na().lu();
            let mut permutation = DMatrix::<f64>::identity(matrix.rows, matrix.rows);
            lu.p().permute_rows(&mut permutation);
            Ok(
                json!({"l":Matrix::from_na(&lu.l())?.json(),"u":Matrix::from_na(&lu.u())?.json(),
                "permutation":Matrix::from_na(&permutation)?.json()}),
            )
        }
        "qr" => {
            let qr = matrix.to_na().qr();
            Ok(json!({"q":Matrix::from_na(&qr.q())?.json(),"r":Matrix::from_na(&qr.r())?.json()}))
        }
        "cholesky" => {
            matrix.square()?;
            for row in 0..matrix.rows {
                for col in row + 1..matrix.cols {
                    let a = matrix.data[row * matrix.cols + col];
                    let b = matrix.data[col * matrix.cols + row];
                    if (a - b).abs() > 1e-10 * a.abs().max(b.abs()).max(1.0) {
                        return Err("Cholesky requires a symmetric matrix.".into());
                    }
                }
            }
            let decomposition = matrix
                .to_na()
                .cholesky()
                .ok_or("Cholesky requires a positive-definite matrix.")?;
            Ok(json!({"l":Matrix::from_na(&decomposition.l())?.json()}))
        }
        "svd" | "pseudoinverse" | "least_squares" => {
            let svd = matrix.to_na().svd(true, true);
            let singular_values = svd.singular_values.as_slice();
            let threshold = linear::tolerance(
                &matrix,
                singular_values,
                input::optional_number(args, "tolerance")?,
            )?;
            match operation {
                "svd" => Ok(
                    json!({"u":Matrix::from_na(svd.u.as_ref().ok_or("SVD U unavailable.")?)?.json(),
                    "singularValues":checked_array(singular_values.to_vec())?,
                    "vT":Matrix::from_na(svd.v_t.as_ref().ok_or("SVD Vt unavailable.")?)?.json(),
                    "rank":singular_values.iter().filter(|v| **v>threshold).count(),"tolerance":threshold}),
                ),
                "pseudoinverse" => {
                    let pseudo = svd.pseudo_inverse(threshold).map_err(str::to_owned)?;
                    Ok(json!({"matrix":Matrix::from_na(&pseudo)?.json(),"tolerance":threshold}))
                }
                _ => {
                    let rhs = input::vector(args, "rhs", &[matrix.rows])?;
                    let solution = svd
                        .solve(&DVector::from_row_slice(&rhs), threshold)
                        .map_err(str::to_owned)?;
                    let result = solution.iter().copied().collect::<Vec<_>>();
                    let residual = matrix.residual(&result, &rhs)?;
                    Ok(
                        json!({"solution":checked_array(result)?,"maxResidual":residual,"tolerance":threshold}),
                    )
                }
            }
        }
        _ => Err("Unsupported matrix factorization.".into()),
    }
}

fn affine_transform(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let dimension = input::int(args, "dimension")?;
    if dimension != 2 && dimension != 3 {
        return Err("Affine dimension must be 2 or 3.".into());
    }
    let dimension = dimension as usize;
    let input_matrix = Matrix::parse(args, "matrix")?;
    let expanded = expand_affine(&input_matrix, dimension)?;
    if operation == "expand" {
        return Ok(json!({"matrix":expanded.json()}));
    }
    if operation == "batch_points" {
        let points = input::points(args, "vectors", dimension, 1, 1024)?;
        let results = points
            .iter()
            .map(|point| affine_apply(&expanded, point, dimension, "transform_point"))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(json!({"vectors":results}));
    }
    let vector = input::vector(args, "vector", &[dimension])?;
    let result = affine_apply(&expanded, &vector, dimension, operation)?;
    Ok(json!({"vector":checked_array(result)?}))
}

fn expand_affine(matrix: &Matrix, dimension: usize) -> Result<Matrix, String> {
    if matrix.rows != dimension && matrix.rows != dimension + 1 {
        return Err("Affine matrix has wrong row count.".into());
    }
    if matrix.cols != dimension + 1 {
        return Err("Affine matrix needs dimension+1 columns.".into());
    }
    let mut data = matrix.data.clone();
    if matrix.rows == dimension {
        data.extend((0..=dimension).map(|i| if i == dimension { 1.0 } else { 0.0 }));
    } else if (0..=dimension).any(|col| {
        (matrix.data[dimension * matrix.cols + col] - if col == dimension { 1.0 } else { 0.0 })
            .abs()
            > 1e-12
    }) {
        return Err("Full affine matrix must have last row [0,...,0,1].".into());
    }
    Matrix::new(dimension + 1, dimension + 1, data)
}

fn affine_apply(
    matrix: &Matrix,
    vector: &[f64],
    dimension: usize,
    operation: &str,
) -> Result<Vec<f64>, String> {
    match operation {
        "transform_point" | "transform_direction" => {
            let mut homogeneous = vector.to_vec();
            homogeneous.push(if operation == "transform_point" {
                1.0
            } else {
                0.0
            });
            Ok(matrix.matvec(&homogeneous)?[..dimension].to_vec())
        }
        "transform_normal" => {
            let linear = Matrix::new(
                dimension,
                dimension,
                (0..dimension)
                    .flat_map(|row| {
                        (0..dimension).map(move |col| matrix.data[row * matrix.cols + col])
                    })
                    .collect(),
            )?;
            math::normalize(&linear.inverse()?.transpose()?.matvec(vector)?)
        }
        _ => Err("Unsupported affine operation.".into()),
    }
}

fn matrix_layout_convert(args: &Value) -> Result<Value, String> {
    let rows = input::int(args, "rows")?;
    let cols = input::int(args, "cols")?;
    if !(1..=32).contains(&rows) || !(1..=32).contains(&cols) {
        return Err("Matrix dimensions must be 1..32.".into());
    }
    let (rows, cols) = (rows as usize, cols as usize);
    let source = input::string(args, "sourceLayout")?;
    let target = input::string(args, "targetLayout")?;
    if !matches!(source, "row_major" | "column_major")
        || !matches!(target, "row_major" | "column_major")
    {
        return Err("Layout must be row_major or column_major.".into());
    }
    let source_minor = if source == "row_major" { cols } else { rows };
    let source_major = if source == "row_major" { rows } else { cols };
    let target_minor = if target == "row_major" { cols } else { rows };
    let target_major = if target == "row_major" { rows } else { cols };
    let source_stride = input::optional_int(args, "sourceStride")?.unwrap_or(source_minor as i32);
    let target_stride = input::optional_int(args, "targetStride")?.unwrap_or(target_minor as i32);
    if source_stride < source_minor as i32
        || target_stride < target_minor as i32
        || source_stride > 1024
        || target_stride > 1024
    {
        return Err("Layout strides must cover each major vector and be <= 1024.".into());
    }
    let input = input::vector(args, "data", &[source_major * source_stride as usize])?;
    let mut output = vec![0.0; target_major * target_stride as usize];
    for row in 0..rows {
        for col in 0..cols {
            let from = if source == "row_major" {
                row * source_stride as usize + col
            } else {
                col * source_stride as usize + row
            };
            let to = if target == "row_major" {
                row * target_stride as usize + col
            } else {
                col * target_stride as usize + row
            };
            output[to] = input[from];
        }
    }
    Ok(
        json!({"rows":rows,"cols":cols,"data":checked_array(output)?,"targetStride":target_stride,"layout":target}),
    )
}
