use crate::math;

pub fn cross2(a: &[f64], b: &[f64]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

pub fn quaternion_multiply(a: &[f64], b: &[f64]) -> Vec<f64> {
    vec![
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

pub fn matrix3_from_quaternion(input: &[f64]) -> Result<Vec<f64>, String> {
    let q = math::normalize(input)?;
    if q.len() != 4 {
        return Err("Quaternion needs four components.".into());
    }
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    Ok(vec![
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y - z * w),
        2.0 * (x * z + y * w),
        2.0 * (x * y + z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z - x * w),
        2.0 * (x * z - y * w),
        2.0 * (y * z + x * w),
        1.0 - 2.0 * (x * x + y * y),
    ])
}

pub fn quaternion_from_matrix3(m: &[f64]) -> Result<Vec<f64>, String> {
    if m.len() != 9 {
        return Err("Rotation matrix needs nine numbers.".into());
    }
    let columns = [
        vec![m[0], m[3], m[6]],
        vec![m[1], m[4], m[7]],
        vec![m[2], m[5], m[8]],
    ];
    if columns
        .iter()
        .any(|column| (math::length(column) - 1.0).abs() > 1e-6)
        || math::dot(&columns[0], &columns[1])?.abs() > 1e-6
        || math::dot(&columns[0], &columns[2])?.abs() > 1e-6
        || math::dot(&columns[1], &columns[2])?.abs() > 1e-6
        || (math::dot(&columns[0], &math::cross(&columns[1], &columns[2])?)? - 1.0).abs() > 1e-6
    {
        return Err("Rotation matrix must be orthonormal with positive determinant.".into());
    }
    let trace = m[0] + m[4] + m[8];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        vec![
            (m[7] - m[5]) / s,
            (m[2] - m[6]) / s,
            (m[3] - m[1]) / s,
            s / 4.0,
        ]
    } else if m[0] > m[4] && m[0] > m[8] {
        let s = (1.0 + m[0] - m[4] - m[8]).sqrt() * 2.0;
        vec![
            s / 4.0,
            (m[1] + m[3]) / s,
            (m[2] + m[6]) / s,
            (m[7] - m[5]) / s,
        ]
    } else if m[4] > m[8] {
        let s = (1.0 + m[4] - m[0] - m[8]).sqrt() * 2.0;
        vec![
            (m[1] + m[3]) / s,
            s / 4.0,
            (m[5] + m[7]) / s,
            (m[2] - m[6]) / s,
        ]
    } else {
        let s = (1.0 + m[8] - m[0] - m[4]).sqrt() * 2.0;
        vec![
            (m[2] + m[6]) / s,
            (m[5] + m[7]) / s,
            s / 4.0,
            (m[3] - m[1]) / s,
        ]
    };
    math::normalize(&q)
}

pub fn quaternion_from_euler_xyz(euler: &[f64]) -> Result<Vec<f64>, String> {
    if euler.len() != 3 {
        return Err("Euler XYZ needs three angles.".into());
    }
    let (sx, cx) = (euler[0] / 2.0).sin_cos();
    let (sy, cy) = (euler[1] / 2.0).sin_cos();
    let (sz, cz) = (euler[2] / 2.0).sin_cos();
    let qx = [sx, 0.0, 0.0, cx];
    let qy = [0.0, sy, 0.0, cy];
    let qz = [0.0, 0.0, sz, cz];
    math::normalize(&quaternion_multiply(&quaternion_multiply(&qz, &qy), &qx))
}

pub fn euler_xyz_from_quaternion(input: &[f64]) -> Result<Vec<f64>, String> {
    let q = math::normalize(input)?;
    if q.len() != 4 {
        return Err("Quaternion needs four components.".into());
    }
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    Ok(vec![
        (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y)),
        (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin(),
        (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z)),
    ])
}

pub fn quaternion_from_axis_angle(axis_angle: &[f64]) -> Result<Vec<f64>, String> {
    if axis_angle.len() != 4 {
        return Err("Axis-angle needs [x,y,z,angle].".into());
    }
    if axis_angle[3] == 0.0 {
        return Ok(vec![0.0, 0.0, 0.0, 1.0]);
    }
    let axis = math::normalize(&axis_angle[..3])?;
    let (sin, cos) = (axis_angle[3] / 2.0).sin_cos();
    Ok(vec![axis[0] * sin, axis[1] * sin, axis[2] * sin, cos])
}

pub fn axis_angle_from_quaternion(input: &[f64]) -> Result<Vec<f64>, String> {
    let q = math::normalize(input)?;
    if q.len() != 4 {
        return Err("Quaternion needs four components.".into());
    }
    let angle = 2.0 * q[3].clamp(-1.0, 1.0).acos();
    let sin_half = (1.0 - q[3] * q[3]).max(0.0).sqrt();
    if sin_half <= math::EPSILON {
        Ok(vec![1.0, 0.0, 0.0, 0.0])
    } else {
        Ok(vec![
            q[0] / sin_half,
            q[1] / sin_half,
            q[2] / sin_half,
            angle,
        ])
    }
}

pub fn plane_distance(plane: &[f64; 4], point: &[f64]) -> f64 {
    plane[0] * point[0] + plane[1] * point[1] + plane[2] * point[2] + plane[3]
}

pub fn frustum_planes(matrix: &[f64], zero_to_one: bool) -> Result<Vec<[f64; 4]>, String> {
    if matrix.len() != 16 {
        return Err("Frustum matrix needs 16 numbers.".into());
    }
    let r0 = [matrix[0], matrix[1], matrix[2], matrix[3]];
    let r1 = [matrix[4], matrix[5], matrix[6], matrix[7]];
    let r2 = [matrix[8], matrix[9], matrix[10], matrix[11]];
    let r3 = [matrix[12], matrix[13], matrix[14], matrix[15]];
    let combine = |a: [f64; 4], b: [f64; 4], sign: f64| -> [f64; 4] {
        [
            a[0] + sign * b[0],
            a[1] + sign * b[1],
            a[2] + sign * b[2],
            a[3] + sign * b[3],
        ]
    };
    let raw = [
        combine(r3, r0, 1.0),
        combine(r3, r0, -1.0),
        combine(r3, r1, 1.0),
        combine(r3, r1, -1.0),
        if zero_to_one {
            r2
        } else {
            combine(r3, r2, 1.0)
        },
        combine(r3, r2, -1.0),
    ];
    raw.into_iter()
        .map(|p| {
            if p.iter().any(|value| !value.is_finite()) {
                return Err("Frustum plane is not finite.".into());
            }
            let length = math::length(&p[..3]);
            if length == 0.0 || !length.is_finite() {
                return Err("Frustum has a degenerate plane.".into());
            }
            Ok([p[0] / length, p[1] / length, p[2] / length, p[3] / length])
        })
        .collect()
}
