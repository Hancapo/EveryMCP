use super::{array, checked_array, value};
use crate::{input, math};
use serde_json::{Value, json};

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "vector_calculate" => vector_calculate(args),
        "matrix_transform" => matrix_transform(args),
        "matrix_multiply" => array(
            "matrix",
            math::multiply4(
                &input::vector(args, "a", &[16])?,
                &input::vector(args, "b", &[16])?,
            )?,
        ),
        "matrix_inverse" => array(
            "matrix",
            math::invert4(&input::vector(args, "matrix", &[16])?)?,
        ),
        "quaternion_rotate" => quaternion_rotate(args),
        "barycentric_2d" => barycentric_2d(args),
        "ray_triangle_intersect" => ray_triangle_intersect(args),
        _ => Err(format!("Unknown tool '{name}'.")),
    }
}

fn vector_calculate(args: &Value) -> Result<Value, String> {
    let operation = input::string(args, "operation")?;
    let a = input::vector(args, "a", &[2, 3])?;
    if operation == "length" {
        return value(math::length(&a));
    }
    if operation == "normalize" {
        return array("vector", math::normalize(&a)?);
    }
    let b = input::vector(args, "b", &[a.len()])?;
    match operation {
        "dot" => value(math::dot(&a, &b)?),
        "cross" => array("vector", math::cross(&a, &b)?),
        "distance" => value(math::length(&math::subtract(&a, &b)?)),
        "angle" => value(
            math::dot(&math::normalize(&a)?, &math::normalize(&b)?)?
                .clamp(-1.0, 1.0)
                .acos(),
        ),
        _ => Err("Unsupported vector operation.".into()),
    }
}

fn matrix_transform(args: &Value) -> Result<Value, String> {
    let matrix = input::vector(args, "matrix", &[16])?;
    let vector = input::vector(args, "vector", &[3])?;
    let kind = input::string(args, "kind")?;
    if kind != "point" && kind != "direction" {
        return Err("'kind' must be 'point' or 'direction'.".into());
    }
    let transformed = math::transform(
        &matrix,
        &[
            vector[0],
            vector[1],
            vector[2],
            if kind == "point" { 1.0 } else { 0.0 },
        ],
    )?;
    if !transformed[3].is_finite() {
        return Err("Result is not finite.".into());
    }
    if kind == "point" {
        if transformed[3].abs() <= math::EPSILON {
            return Err("Homogeneous w is zero.".into());
        }
        Ok(
            json!({"vector":checked_array(vec![transformed[0]/transformed[3],transformed[1]/transformed[3],transformed[2]/transformed[3]])?,"w":transformed[3]}),
        )
    } else {
        Ok(json!({"vector":checked_array(transformed[..3].to_vec())?,"w":transformed[3]}))
    }
}

fn quaternion_rotate(args: &Value) -> Result<Value, String> {
    let q = math::normalize(&input::vector(args, "quaternion", &[4])?)?;
    let v = input::vector(args, "vector", &[3])?;
    let u = &q[..3];
    let mut t = math::cross(u, &v)?;
    t.iter_mut().for_each(|x| *x *= 2.0);
    let first = math::add_scaled(&v, &t, q[3])?;
    array(
        "vector",
        math::add_scaled(&first, &math::cross(u, &t)?, 1.0)?,
    )
}

fn barycentric_2d(args: &Value) -> Result<Value, String> {
    let p = input::vector(args, "point", &[2])?;
    let a = input::vector(args, "a", &[2])?;
    let b = input::vector(args, "b", &[2])?;
    let c = input::vector(args, "c", &[2])?;
    let v0 = math::subtract(&b, &a)?;
    let v1 = math::subtract(&c, &a)?;
    let v2 = math::subtract(&p, &a)?;
    let determinant = v0[0] * v1[1] - v0[1] * v1[0];
    if determinant.abs() <= math::EPSILON {
        return Err("Triangle is degenerate.".into());
    }
    let w1 = (v2[0] * v1[1] - v2[1] * v1[0]) / determinant;
    let w2 = (v0[0] * v2[1] - v0[1] * v2[0]) / determinant;
    let w0 = 1.0 - w1 - w2;
    let inside = w0 >= -math::EPSILON && w1 >= -math::EPSILON && w2 >= -math::EPSILON;
    Ok(json!({"weights":checked_array(vec![w0,w1,w2])?,"inside":inside}))
}

fn ray_triangle_intersect(args: &Value) -> Result<Value, String> {
    let origin = input::vector(args, "origin", &[3])?;
    let direction = math::normalize(&input::vector(args, "direction", &[3])?)?;
    let a = input::vector(args, "a", &[3])?;
    let b = input::vector(args, "b", &[3])?;
    let c = input::vector(args, "c", &[3])?;
    let edge1 = math::subtract(&b, &a)?;
    let edge2 = math::subtract(&c, &a)?;
    let p = math::cross(&direction, &edge2)?;
    let determinant = math::dot(&edge1, &p)?;
    let cull = input::optional_bool(args, "cullBackface")?;
    if if cull {
        determinant <= math::EPSILON
    } else {
        determinant.abs() <= math::EPSILON
    } {
        return Ok(json!({"hit":false}));
    }
    let inv = 1.0 / determinant;
    let t = math::subtract(&origin, &a)?;
    let u = math::dot(&t, &p)? * inv;
    if u < -math::EPSILON || u > 1.0 + math::EPSILON {
        return Ok(json!({"hit":false}));
    }
    let q = math::cross(&t, &edge1)?;
    let v = math::dot(&direction, &q)? * inv;
    if v < -math::EPSILON || u + v > 1.0 + math::EPSILON {
        return Ok(json!({"hit":false}));
    }
    let distance = math::dot(&edge2, &q)? * inv;
    if distance < 0.0 {
        return Ok(json!({"hit":false}));
    }
    if !distance.is_finite() {
        return Err("Result is not finite.".into());
    }
    Ok(json!({"hit":true,"distance":distance,
        "point":checked_array(math::add_scaled(&origin, &direction, distance)?)?,
        "weights":checked_array(vec![1.0-u-v,u,v])?}))
}
