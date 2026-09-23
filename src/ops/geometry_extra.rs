use super::checked_array;
use crate::{geometry_math as gm, input, math};
use serde_json::{Value, json};

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "compose_transform" => compose_transform(args),
        "decompose_transform" => decompose_transform(args),
        "rotation_convert" => rotation_convert(args),
        "quaternion_slerp" => quaternion_slerp(args),
        "project_unproject" => project_unproject(args),
        "ray_primitive_intersect" => ray_primitive_intersect(args),
        "closest_point" => closest_point(args),
        "segment_intersect_2d" => segment_intersect_2d(args),
        "polygon_measure_2d" => polygon_measure_2d(args),
        "frustum_test" => frustum_test(args),
        _ => Err(format!("Unknown tool '{name}'.")),
    }
}

fn compose_transform(args: &Value) -> Result<Value, String> {
    let translation = input::vector(args, "translation", &[3])?;
    let rotation = input::vector(args, "rotation", &[4])?;
    let scale = input::vector(args, "scale", &[3])?;
    let r = gm::matrix3_from_quaternion(&rotation)?;
    let mut matrix = vec![0.0; 16];
    for row in 0..3 {
        for col in 0..3 {
            matrix[row * 4 + col] = r[row * 3 + col] * scale[col];
        }
        matrix[row * 4 + 3] = translation[row];
    }
    matrix[15] = 1.0;
    Ok(json!({"matrix":checked_array(matrix)?}))
}

fn decompose_transform(args: &Value) -> Result<Value, String> {
    let m = input::vector(args, "matrix", &[16])?;
    if m[12].abs() > math::EPSILON
        || m[13].abs() > math::EPSILON
        || m[14].abs() > math::EPSILON
        || (m[15] - 1.0).abs() > math::EPSILON
    {
        return Err("Matrix must be affine with last row [0,0,0,1].".into());
    }
    let translation = vec![m[3], m[7], m[11]];
    let col0 = vec![m[0], m[4], m[8]];
    let col1 = vec![m[1], m[5], m[9]];
    let col2 = vec![m[2], m[6], m[10]];
    let mut sx = math::length(&col0);
    if sx == 0.0 {
        return Err("Matrix has zero scale.".into());
    }
    let mut u0 = math::normalize(&col0)?;
    let shear_xy_raw = math::dot(&u0, &col1)?;
    let y_orthogonal = math::add_scaled(&col1, &u0, -shear_xy_raw)?;
    let sy = math::length(&y_orthogonal);
    if sy == 0.0 {
        return Err("Matrix has zero scale.".into());
    }
    let u1 = math::normalize(&y_orthogonal)?;
    let shear_xz_raw = math::dot(&u0, &col2)?;
    let z_partial = math::add_scaled(&col2, &u0, -shear_xz_raw)?;
    let shear_yz_raw = math::dot(&u1, &z_partial)?;
    let z_orthogonal = math::add_scaled(&z_partial, &u1, -shear_yz_raw)?;
    let sz = math::length(&z_orthogonal);
    if sz == 0.0 {
        return Err("Matrix has zero scale.".into());
    }
    let u2 = math::normalize(&z_orthogonal)?;
    let mut shear_xy = shear_xy_raw / sy;
    let mut shear_xz = shear_xz_raw / sz;
    let shear_yz = shear_yz_raw / sz;
    let reflection = math::dot(&u0, &math::cross(&u1, &u2)?)? < 0.0;
    if reflection {
        sx = -sx;
        u0.iter_mut().for_each(|x| *x = -*x);
        shear_xy = -shear_xy;
        shear_xz = -shear_xz;
    }
    let rotation_matrix = vec![
        u0[0], u1[0], u2[0], u0[1], u1[1], u2[1], u0[2], u1[2], u2[2],
    ];
    let rotation = gm::quaternion_from_matrix3(&rotation_matrix)?;
    Ok(
        json!({"translation":checked_array(translation)?,"rotation":checked_array(rotation)?,
        "scale":checked_array(vec![sx,sy,sz])?,"shear":checked_array(vec![shear_xy,shear_xz,shear_yz])?,
        "reflection":reflection}),
    )
}

fn rotation_convert(args: &Value) -> Result<Value, String> {
    let from = input::string(args, "from")?;
    let to = input::string(args, "to")?;
    if args.get("order").is_some() && input::string(args, "order")? != "XYZ" {
        return Err("Only Euler order XYZ is supported.".into());
    }
    let input_value = input::vector(args, "value", &[])?;
    let q = match from {
        "quaternion" if input_value.len() == 4 => math::normalize(&input_value)?,
        "axis_angle" => gm::quaternion_from_axis_angle(&input_value)?,
        "euler" => gm::quaternion_from_euler_xyz(&input_value)?,
        "matrix" => gm::quaternion_from_matrix3(&input_value)?,
        _ => return Err("Unsupported rotation source or component count.".into()),
    };
    let output = match to {
        "quaternion" => q,
        "axis_angle" => gm::axis_angle_from_quaternion(&q)?,
        "euler" => gm::euler_xyz_from_quaternion(&q)?,
        "matrix" => gm::matrix3_from_quaternion(&q)?,
        _ => return Err("Unsupported rotation target.".into()),
    };
    Ok(json!({"value":checked_array(output)?}))
}

fn quaternion_slerp(args: &Value) -> Result<Value, String> {
    let a = math::normalize(&input::vector(args, "a", &[4])?)?;
    let mut b = math::normalize(&input::vector(args, "b", &[4])?)?;
    let t = input::number(args, "t")?;
    if !(0.0..=1.0).contains(&t) {
        return Err("t must be in [0,1].".into());
    }
    let mut dot = math::dot(&a, &b)?;
    if dot < 0.0 {
        b.iter_mut().for_each(|x| *x = -*x);
        dot = -dot;
    }
    let result = if dot > 0.9995 {
        math::normalize(&math::add_scaled(&a, &math::subtract(&b, &a)?, t)?)?
    } else {
        let theta = dot.clamp(-1.0, 1.0).acos();
        let sin_theta = theta.sin();
        let wa = ((1.0 - t) * theta).sin() / sin_theta;
        let wb = (t * theta).sin() / sin_theta;
        math::normalize(
            &a.iter()
                .zip(&b)
                .map(|(x, y)| x * wa + y * wb)
                .collect::<Vec<_>>(),
        )?
    };
    Ok(json!({"quaternion":checked_array(result)?}))
}

fn project_unproject(args: &Value) -> Result<Value, String> {
    let mode = input::string(args, "mode")?;
    let matrix = input::vector(args, "matrix", &[16])?;
    let viewport = input::vector(args, "viewport", &[4])?;
    let vector = input::vector(args, "vector", &[3])?;
    let depth = input::string(args, "depthRange")?;
    if !matches!(depth, "zero_to_one" | "minus_one_to_one") {
        return Err("Invalid depthRange.".into());
    }
    if viewport[2] <= 0.0 || viewport[3] <= 0.0 {
        return Err("Viewport width and height must be positive.".into());
    }
    let result = match mode {
        "project" => {
            let clip = math::transform(&matrix, &[vector[0], vector[1], vector[2], 1.0])?;
            if !clip[3].is_finite() || clip[3].abs() <= math::EPSILON {
                return Err("Projection w is zero.".into());
            }
            let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
            vec![
                viewport[0] + (ndc[0] + 1.0) * viewport[2] / 2.0,
                viewport[1] + (1.0 - ndc[1]) * viewport[3] / 2.0,
                if depth == "zero_to_one" {
                    ndc[2]
                } else {
                    (ndc[2] + 1.0) / 2.0
                },
            ]
        }
        "unproject" => {
            let inverse = math::invert4(&matrix)?;
            let ndc = [
                2.0 * (vector[0] - viewport[0]) / viewport[2] - 1.0,
                1.0 - 2.0 * (vector[1] - viewport[1]) / viewport[3],
                if depth == "zero_to_one" {
                    vector[2]
                } else {
                    2.0 * vector[2] - 1.0
                },
            ];
            let world = math::transform(&inverse, &[ndc[0], ndc[1], ndc[2], 1.0])?;
            if !world[3].is_finite() || world[3].abs() <= math::EPSILON {
                return Err("Unprojection w is zero.".into());
            }
            vec![
                world[0] / world[3],
                world[1] / world[3],
                world[2] / world[3],
            ]
        }
        _ => return Err("Mode must be project or unproject.".into()),
    };
    Ok(json!({"vector":checked_array(result)?}))
}

fn ray_primitive_intersect(args: &Value) -> Result<Value, String> {
    let primitive = input::string(args, "primitive")?;
    let origin = input::vector(args, "origin", &[3])?;
    let direction = math::normalize(&input::vector(args, "direction", &[3])?)?;
    let hit = match primitive {
        "plane" => {
            let raw_normal = input::vector(args, "normal", &[3])?;
            let normal_length = math::length(&raw_normal);
            if normal_length == 0.0 || !normal_length.is_finite() {
                return Err("Plane normal must have finite nonzero length.".into());
            }
            let normal = math::normalize(&raw_normal)?;
            let offset = input::number(args, "distance")? / normal_length;
            let denominator = math::dot(&normal, &direction)?;
            if denominator.abs() <= math::EPSILON {
                None
            } else {
                let t = -(math::dot(&normal, &origin)? + offset) / denominator;
                if t < 0.0 { None } else { Some((t, normal)) }
            }
        }
        "sphere" => {
            let center = input::vector(args, "center", &[3])?;
            let radius = input::number(args, "radius")?;
            if radius <= 0.0 {
                return Err("Sphere radius must be positive.".into());
            }
            let oc = math::subtract(&origin, &center)?;
            let b = math::dot(&oc, &direction)?;
            let c = math::dot(&oc, &oc)? - radius * radius;
            let discriminant = b * b - c;
            if discriminant < 0.0 {
                None
            } else {
                let root = discriminant.sqrt();
                let t = if -b - root >= 0.0 {
                    -b - root
                } else {
                    -b + root
                };
                if t < 0.0 {
                    None
                } else {
                    let point = math::add_scaled(&origin, &direction, t)?;
                    Some((t, math::normalize(&math::subtract(&point, &center)?)?))
                }
            }
        }
        "aabb" => {
            let minimum = input::vector(args, "min", &[3])?;
            let maximum = input::vector(args, "max", &[3])?;
            if (0..3).any(|i| minimum[i] > maximum[i]) {
                return Err("AABB min must not exceed max.".into());
            }
            ray_aabb(&origin, &direction, &minimum, &maximum)
        }
        _ => return Err("Unsupported ray primitive.".into()),
    };
    if let Some((distance, normal)) = hit {
        if !distance.is_finite() {
            return Err("Result is not finite.".into());
        }
        Ok(
            json!({"hit":true,"distance":distance,"point":checked_array(math::add_scaled(&origin,&direction,distance)?)?,
            "normal":checked_array(normal)?}),
        )
    } else {
        Ok(json!({"hit":false}))
    }
}

fn ray_aabb(
    origin: &[f64],
    direction: &[f64],
    minimum: &[f64],
    maximum: &[f64],
) -> Option<(f64, Vec<f64>)> {
    let (mut entry, mut exit) = (f64::NEG_INFINITY, f64::INFINITY);
    let (mut entry_normal, mut exit_normal) = (vec![0.0; 3], vec![0.0; 3]);
    for axis in 0..3 {
        if direction[axis] == 0.0 {
            if origin[axis] < minimum[axis] || origin[axis] > maximum[axis] {
                return None;
            }
            continue;
        }
        let mut near = (minimum[axis] - origin[axis]) / direction[axis];
        let mut far = (maximum[axis] - origin[axis]) / direction[axis];
        let mut near_sign = -1.0;
        if near > far {
            std::mem::swap(&mut near, &mut far);
            near_sign = 1.0;
        }
        if near > entry {
            entry = near;
            entry_normal = vec![0.0; 3];
            entry_normal[axis] = near_sign;
        }
        if far < exit {
            exit = far;
            exit_normal = vec![0.0; 3];
            exit_normal[axis] = -near_sign;
        }
        if entry > exit {
            return None;
        }
    }
    if exit < 0.0 {
        None
    } else if entry >= 0.0 {
        Some((entry, entry_normal))
    } else {
        Some((exit, exit_normal))
    }
}

fn closest_point(args: &Value) -> Result<Value, String> {
    let primitive = input::string(args, "primitive")?;
    let point = input::vector(args, "point", &[3])?;
    let nearest = match primitive {
        "segment" => closest_segment(
            &point,
            &input::vector(args, "a", &[3])?,
            &input::vector(args, "b", &[3])?,
        )?,
        "triangle" => closest_triangle(
            &point,
            &input::vector(args, "a", &[3])?,
            &input::vector(args, "b", &[3])?,
            &input::vector(args, "c", &[3])?,
        )?,
        "aabb" => {
            let minimum = input::vector(args, "min", &[3])?;
            let maximum = input::vector(args, "max", &[3])?;
            if (0..3).any(|i| minimum[i] > maximum[i]) {
                return Err("AABB min must not exceed max.".into());
            }
            (0..3)
                .map(|i| point[i].clamp(minimum[i], maximum[i]))
                .collect()
        }
        _ => return Err("Unsupported closest-point primitive.".into()),
    };
    let distance = math::length(&math::subtract(&point, &nearest)?);
    if !distance.is_finite() {
        return Err("Result is not finite.".into());
    }
    Ok(json!({"point":checked_array(nearest)?,"distance":distance}))
}

fn closest_segment(point: &[f64], a: &[f64], b: &[f64]) -> Result<Vec<f64>, String> {
    let ab = math::subtract(b, a)?;
    let squared = math::dot(&ab, &ab)?;
    if squared == 0.0 {
        return Ok(a.to_vec());
    }
    let t = (math::dot(&math::subtract(point, a)?, &ab)? / squared).clamp(0.0, 1.0);
    math::add_scaled(a, &ab, t)
}

fn closest_triangle(point: &[f64], a: &[f64], b: &[f64], c: &[f64]) -> Result<Vec<f64>, String> {
    let ab = math::subtract(b, a)?;
    let ac = math::subtract(c, a)?;
    if math::length(&math::cross(&ab, &ac)?) == 0.0 {
        return Err("Triangle is degenerate.".into());
    }
    let ap = math::subtract(point, a)?;
    let d1 = math::dot(&ab, &ap)?;
    let d2 = math::dot(&ac, &ap)?;
    if d1 <= 0.0 && d2 <= 0.0 {
        return Ok(a.to_vec());
    }
    let bp = math::subtract(point, b)?;
    let d3 = math::dot(&ab, &bp)?;
    let d4 = math::dot(&ac, &bp)?;
    if d3 >= 0.0 && d4 <= d3 {
        return Ok(b.to_vec());
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return math::add_scaled(a, &ab, d1 / (d1 - d3));
    }
    let cp = math::subtract(point, c)?;
    let d5 = math::dot(&ab, &cp)?;
    let d6 = math::dot(&ac, &cp)?;
    if d6 >= 0.0 && d5 <= d6 {
        return Ok(c.to_vec());
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return math::add_scaled(a, &ac, d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return math::add_scaled(
            b,
            &math::subtract(c, b)?,
            (d4 - d3) / ((d4 - d3) + (d5 - d6)),
        );
    }
    let denominator = va + vb + vc;
    if denominator == 0.0 {
        return Err("Triangle is degenerate.".into());
    }
    let v = vb / denominator;
    let w = vc / denominator;
    math::add_scaled(&math::add_scaled(a, &ab, v)?, &ac, w)
}

fn segment_intersect_2d(args: &Value) -> Result<Value, String> {
    let p = input::vector(args, "p1", &[2])?;
    let p2 = input::vector(args, "p2", &[2])?;
    let q = input::vector(args, "q1", &[2])?;
    let q2 = input::vector(args, "q2", &[2])?;
    let r = math::subtract(&p2, &p)?;
    let s = math::subtract(&q2, &q)?;
    let qp = math::subtract(&q, &p)?;
    let rxs = gm::cross2(&r, &s);
    if rxs.abs() <= math::EPSILON {
        if gm::cross2(&qp, &r).abs() > math::EPSILON {
            return Ok(json!({"kind":"none"}));
        }
        let rr = math::dot(&r, &r)?;
        if rr == 0.0 {
            let closest = closest_segment(&p, &q, &q2)?;
            return if math::length(&math::subtract(&p, &closest)?) <= math::EPSILON {
                Ok(json!({"kind":"point","point":checked_array(p)?}))
            } else {
                Ok(json!({"kind":"none"}))
            };
        }
        let t0 = math::dot(&qp, &r)? / rr;
        let t1 = t0 + math::dot(&s, &r)? / rr;
        let start = t0.min(t1).max(0.0);
        let end = t0.max(t1).min(1.0);
        if start > end + math::EPSILON {
            return Ok(json!({"kind":"none"}));
        }
        if (end - start).abs() <= math::EPSILON {
            return Ok(
                json!({"kind":"point","point":checked_array(math::add_scaled(&p,&r,start)?)?}),
            );
        }
        return Ok(
            json!({"kind":"overlap","segment":[checked_array(math::add_scaled(&p,&r,start)?)?,
            checked_array(math::add_scaled(&p,&r,end)?)?]}),
        );
    }
    let t = gm::cross2(&qp, &s) / rxs;
    let u = gm::cross2(&qp, &r) / rxs;
    if t < -math::EPSILON
        || t > 1.0 + math::EPSILON
        || u < -math::EPSILON
        || u > 1.0 + math::EPSILON
    {
        return Ok(json!({"kind":"none"}));
    }
    Ok(json!({"kind":"point","point":checked_array(math::add_scaled(&p,&r,t.clamp(0.0,1.0))?)?}))
}

fn polygon_measure_2d(args: &Value) -> Result<Value, String> {
    let points = input::points(args, "points", 2, 3, 10000)?;
    let mut cross_sum = 0.0;
    let (mut cx, mut cy) = (0.0, 0.0);
    for index in 0..points.len() {
        let a = &points[index];
        let b = &points[(index + 1) % points.len()];
        let cross = gm::cross2(a, b);
        cross_sum += cross;
        cx += (a[0] + b[0]) * cross;
        cy += (a[1] + b[1]) * cross;
    }
    if cross_sum.abs() <= math::EPSILON {
        return Err("Polygon has zero signed area.".into());
    }
    let centroid = vec![cx / (3.0 * cross_sum), cy / (3.0 * cross_sum)];
    let signed_area = cross_sum / 2.0;
    if !signed_area.is_finite() {
        return Err("Result is not finite.".into());
    }
    Ok(
        json!({"signedArea":signed_area,"area":signed_area.abs(),"centroid":checked_array(centroid)?,
        "winding":if signed_area>0.0 {"ccw"} else {"cw"}}),
    )
}

fn frustum_test(args: &Value) -> Result<Value, String> {
    let matrix = input::vector(args, "matrix", &[16])?;
    let primitive = input::string(args, "primitive")?;
    let depth = input::string(args, "depthRange")?;
    if !matches!(depth, "zero_to_one" | "minus_one_to_one") {
        return Err("Invalid depthRange.".into());
    }
    let planes = gm::frustum_planes(&matrix, depth == "zero_to_one")?;
    let (center, radius) = match primitive {
        "point" => (input::vector(args, "point", &[3])?, vec![0.0; 3]),
        "sphere" => {
            let radius = input::number(args, "radius")?;
            if radius < 0.0 {
                return Err("Sphere radius must be nonnegative.".into());
            }
            (input::vector(args, "center", &[3])?, vec![radius; 3])
        }
        "aabb" => {
            let minimum = input::vector(args, "min", &[3])?;
            let maximum = input::vector(args, "max", &[3])?;
            if (0..3).any(|i| minimum[i] > maximum[i]) {
                return Err("AABB min must not exceed max.".into());
            }
            (
                (0..3).map(|i| (minimum[i] + maximum[i]) / 2.0).collect(),
                (0..3).map(|i| (maximum[i] - minimum[i]) / 2.0).collect(),
            )
        }
        _ => return Err("Unsupported frustum primitive.".into()),
    };
    let mut intersect = false;
    for plane in planes {
        let distance = gm::plane_distance(&plane, &center);
        let extent = if primitive == "aabb" {
            plane[..3]
                .iter()
                .zip(&radius)
                .map(|(n, r)| n.abs() * r)
                .sum::<f64>()
        } else if primitive == "sphere" {
            radius[0]
        } else {
            0.0
        };
        if distance < -extent {
            return Ok(json!({"classification":"outside"}));
        }
        if distance < extent {
            intersect = true;
        }
    }
    Ok(json!({"classification":if intersect {"intersect"} else {"inside"}}))
}
