use super::checked_array;
use crate::geometry_math as gm;
use crate::input;
use nalgebra::{Matrix3, SymmetricEigen};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

type P3 = [f64; 3];

const MAX_POINTS: usize = 100_000;
const MAX_HULL_3D_POINTS: usize = 10_000;
const MAX_POLYGON_VERTICES: usize = 5_000;
const MAX_MESH_VERTICES: usize = 200_000;
const MAX_MESH_TRIANGLES: usize = 400_000;
const MAX_REPORTED: usize = 100;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "bounding_volume" => bounding_volume(args),
        "mesh_inspect" => mesh_inspect(args),
        "convex_hull" => convex_hull(args),
        "polygon_triangulate" => polygon_triangulate(args),
        _ => unreachable!(),
    }
}

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: P3, s: f64) -> P3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm2(a: P3) -> f64 {
    dot(a, a)
}

fn points3(args: &Value, key: &str, max: usize) -> Result<Vec<P3>, String> {
    Ok(input::points(args, key, 3, 1, max)?
        .into_iter()
        .map(|p| [p[0], p[1], p[2]])
        .collect())
}

fn bounds(points: &[P3]) -> (P3, P3) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in points {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    (lo, hi)
}

/// Squared tolerance scaled to the point cloud's extent.
fn tolerance(points: &[P3]) -> f64 {
    let (lo, hi) = bounds(points);
    let diagonal = norm2(sub(hi, lo)).sqrt().max(1e-300);
    (diagonal * 1e-9).powi(2)
}

#[derive(Clone, Copy)]
struct Ball {
    center: P3,
    radius2: f64,
}

impl Ball {
    fn contains(&self, p: P3, eps2: f64) -> bool {
        norm2(sub(p, self.center)) <= self.radius2 + eps2 + self.radius2 * 1e-12
    }
}

fn ball2(a: P3, b: P3) -> Ball {
    Ball {
        center: scale(add(a, b), 0.5),
        radius2: norm2(sub(a, b)) / 4.0,
    }
}

/// Smallest ball with `a`, `b`, `c` on its boundary (falls back for collinear points).
fn ball3(a: P3, b: P3, c: P3, eps2: f64) -> Ball {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let n = cross(ab, ac);
    let n2 = norm2(n);
    if n2 <= eps2 * (norm2(ab) + norm2(ac)).max(1e-300) {
        // Collinear: the ball over the farthest pair.
        return [ball2(a, b), ball2(a, c), ball2(b, c)]
            .into_iter()
            .max_by(|x, y| x.radius2.total_cmp(&y.radius2))
            .unwrap();
    }
    let offset = scale(
        add(
            scale(cross(n, ab), norm2(ac)),
            scale(cross(ac, n), norm2(ab)),
        ),
        1.0 / (2.0 * n2),
    );
    Ball {
        center: add(a, offset),
        radius2: norm2(offset),
    }
}

/// Smallest ball with four points on its boundary; coplanar sets use the best
/// three-point ball that contains all four.
fn ball4(a: P3, b: P3, c: P3, d: P3, eps2: f64) -> Ball {
    let rows = [sub(b, a), sub(c, a), sub(d, a)];
    let m = Matrix3::from_rows(&rows.map(|r| nalgebra::RowVector3::new(r[0], r[1], r[2])));
    let rhs = nalgebra::Vector3::new(
        norm2(rows[0]) / 2.0,
        norm2(rows[1]) / 2.0,
        norm2(rows[2]) / 2.0,
    );
    let det = m.determinant();
    let size = rows.iter().map(|r| norm2(*r)).fold(0.0, f64::max);
    if det.abs() > 1e-12 * size.powf(1.5)
        && let Some(solution) = m.lu().solve(&rhs)
    {
        let offset = [solution[0], solution[1], solution[2]];
        return Ball {
            center: add(a, offset),
            radius2: norm2(offset),
        };
    }
    let all = [a, b, c, d];
    [(a, b, c), (a, b, d), (a, c, d), (b, c, d)]
        .into_iter()
        .map(|(x, y, z)| ball3(x, y, z, eps2))
        .filter(|ball| all.iter().all(|&p| ball.contains(p, eps2)))
        .min_by(|x, y| x.radius2.total_cmp(&y.radius2))
        .unwrap_or_else(|| ball3(a, b, c, eps2))
}

/// Exact minimum enclosing sphere (incremental Welzl with a deterministic shuffle).
fn min_sphere(points: &[P3]) -> Ball {
    let eps2 = tolerance(points);
    let mut order: Vec<usize> = (0..points.len()).collect();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for i in (1..order.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        order.swap(i, (state % (i as u64 + 1)) as usize);
    }
    let p: Vec<P3> = order.iter().map(|&i| points[i]).collect();
    let mut ball = Ball {
        center: p[0],
        radius2: 0.0,
    };
    for i in 1..p.len() {
        if ball.contains(p[i], eps2) {
            continue;
        }
        ball = Ball {
            center: p[i],
            radius2: 0.0,
        };
        for j in 0..i {
            if ball.contains(p[j], eps2) {
                continue;
            }
            ball = ball2(p[i], p[j]);
            for k in 0..j {
                if ball.contains(p[k], eps2) {
                    continue;
                }
                ball = ball3(p[i], p[j], p[k], eps2);
                for l in 0..k {
                    if !ball.contains(p[l], eps2) {
                        ball = ball4(p[i], p[j], p[k], p[l], eps2);
                    }
                }
            }
        }
    }
    ball
}

fn outer(a: P3) -> Matrix3<f64> {
    let v = nalgebra::Vector3::from(a);
    v * v.transpose()
}

fn point_covariance(points: &[P3]) -> Matrix3<f64> {
    let n = points.len() as f64;
    let mean = scale(points.iter().fold([0.0; 3], |acc, &p| add(acc, p)), 1.0 / n);
    points
        .iter()
        .fold(Matrix3::zeros(), |acc, &p| acc + outer(sub(p, mean)))
        / n
}

/// Covariance of the hull surface itself (area-weighted over its triangles), which
/// does not depend on how densely the input samples each region.
fn surface_covariance(points: &[P3], faces: &[[usize; 3]]) -> Option<Matrix3<f64>> {
    let (mut total, mut first, mut second) = (0.0, [0.0; 3], Matrix3::zeros());
    for face in faces {
        let [a, b, c] = face.map(|v| points[v]);
        let area = norm2(cross(sub(b, a), sub(c, a))).sqrt() / 2.0;
        let centroid = scale(add(add(a, b), c), 1.0 / 3.0);
        total += area;
        first = add(first, scale(centroid, area));
        second += (outer(centroid) * 9.0 + outer(a) + outer(b) + outer(c)) * (area / 12.0);
    }
    if total <= 0.0 {
        return None;
    }
    let mean = scale(first, 1.0 / total);
    Some(second / total - outer(mean))
}

/// Tightest box along the given orthonormal axes.
fn fit_box(points: &[P3], axes: [P3; 3], method: &str) -> Result<(f64, Value), String> {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for &p in points {
        for i in 0..3 {
            let t = dot(p, axes[i]);
            lo[i] = lo[i].min(t);
            hi[i] = hi[i].max(t);
        }
    }
    let mut center = [0.0; 3];
    for i in 0..3 {
        center = add(center, scale(axes[i], (lo[i] + hi[i]) / 2.0));
    }
    let half: Vec<f64> = (0..3).map(|i| (hi[i] - lo[i]) / 2.0).collect();
    let rotation = gm::quaternion_from_matrix3(&[
        axes[0][0], axes[1][0], axes[2][0], axes[0][1], axes[1][1], axes[2][1], axes[0][2],
        axes[1][2], axes[2][2],
    ])?;
    let volume = 8.0 * half[0] * half[1] * half[2];
    Ok((
        volume,
        json!({"method":method,"center":checked_array(center.to_vec())?,
            "axes":axes.iter().map(|a| checked_array(a.to_vec())).collect::<Result<Vec<_>,_>>()?,
            "halfExtents":checked_array(half)?,"rotation":checked_array(rotation)?,"volume":volume}),
    ))
}

/// PCA-fitted OBB (on the hull surface when the hull can be built), or the
/// axis-aligned box when that is smaller.
fn obb(points: &[P3]) -> Result<Value, String> {
    let surface = (points.len() <= MAX_HULL_3D_POINTS)
        .then(|| hull_faces(points).ok())
        .flatten()
        .and_then(|faces| surface_covariance(points, &faces));
    let (covariance, method) = match surface {
        Some(covariance) => (covariance, "pca_hull_surface"),
        None => (point_covariance(points), "pca_points"),
    };
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let fitted = fit_box(points, pca_axes(covariance).unwrap_or(identity), method)?;
    let aligned = fit_box(points, identity, "axis_aligned")?;
    Ok(if aligned.0 < fitted.0 * (1.0 - 1e-12) {
        aligned.1
    } else {
        fitted.1
    })
}

fn pca_axes(covariance: Matrix3<f64>) -> Option<[P3; 3]> {
    let eigen = SymmetricEigen::new(covariance);
    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| eigen.eigenvalues[b].total_cmp(&eigen.eigenvalues[a]));
    let column = |i: usize| -> P3 {
        let c = eigen.eigenvectors.column(i);
        [c[0], c[1], c[2]]
    };
    let mut axes = [column(order[0]), column(order[1]), [0.0; 3]];
    if !axes[..2].iter().flatten().all(|v| v.is_finite()) {
        return None;
    }
    axes[0] = scale(axes[0], 1.0 / norm2(axes[0]).sqrt());
    // Re-orthogonalize and make the frame right-handed.
    axes[1] = sub(axes[1], scale(axes[0], dot(axes[1], axes[0])));
    let length = norm2(axes[1]).sqrt();
    // Also rejects NaN.
    if length.is_nan() || length <= 1e-9 {
        return None;
    }
    axes[1] = scale(axes[1], 1.0 / length);
    axes[2] = cross(axes[0], axes[1]);
    Some(axes)
}

fn bounding_volume(args: &Value) -> Result<Value, String> {
    let points = points3(args, "points", MAX_POINTS)?;
    let kind = args.get("kind").and_then(Value::as_str).unwrap_or("all");
    if !matches!(kind, "all" | "aabb" | "sphere" | "obb") {
        return Err("'kind' must be all, aabb, sphere or obb.".into());
    }
    let mut result = json!({"pointCount":points.len()});
    if matches!(kind, "all" | "aabb") {
        let (lo, hi) = bounds(&points);
        let size = sub(hi, lo);
        result["aabb"] = json!({"min":checked_array(lo.to_vec())?,"max":checked_array(hi.to_vec())?,
            "center":checked_array(scale(add(lo, hi), 0.5).to_vec())?,"halfExtents":checked_array(scale(size, 0.5).to_vec())?,
            "volume":size[0] * size[1] * size[2],
            "surfaceArea":2.0 * (size[0] * size[1] + size[1] * size[2] + size[0] * size[2])});
    }
    if matches!(kind, "all" | "sphere") {
        let ball = min_sphere(&points);
        result["sphere"] = json!({"method":"minimum_enclosing","center":checked_array(ball.center.to_vec())?,
            "radius":ball.radius2.sqrt()});
    }
    if matches!(kind, "all" | "obb") {
        result["obb"] = obb(&points)?;
    }
    Ok(result)
}

fn indices(args: &Value, key: &str, vertices: usize, max: usize) -> Result<Vec<usize>, String> {
    let raw = input::vector(args, key, &[])?;
    if raw.len() > max {
        return Err(format!("'{key}' may contain at most {max} entries."));
    }
    raw.into_iter()
        .map(|v| {
            if v.fract() != 0.0 || v < 0.0 || v as usize >= vertices {
                Err(format!(
                    "'{key}' must contain integers below the vertex count {vertices}."
                ))
            } else {
                Ok(v as usize)
            }
        })
        .collect()
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn find(&mut self, mut x: usize) -> usize {
        while self.0[x] != x {
            self.0[x] = self.0[self.0[x]];
            x = self.0[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a] = b;
        }
    }
}

fn mesh_inspect(args: &Value) -> Result<Value, String> {
    let vertices = points3(args, "vertices", MAX_MESH_VERTICES)?;
    let triangles: Vec<[usize; 3]> = match args.get("indices") {
        Some(_) => {
            let flat = indices(args, "indices", vertices.len(), MAX_MESH_TRIANGLES * 3)?;
            if flat.len() % 3 != 0 {
                return Err("'indices' length must be a multiple of 3.".into());
            }
            flat.as_chunks::<3>().0.to_vec()
        }
        None => {
            if vertices.len() % 3 != 0 {
                return Err("Without 'indices', the vertex count must be a multiple of 3.".into());
            }
            (0..vertices.len() / 3)
                .map(|t| [3 * t, 3 * t + 1, 3 * t + 2])
                .collect()
        }
    };
    let weld = input::optional_number(args, "weldTolerance")?.unwrap_or(0.0);
    if !(weld >= 0.0 && weld.is_finite()) {
        return Err("'weldTolerance' must be a nonnegative number.".into());
    }
    let include_normals = input::optional_bool(args, "includeNormals")?;
    if include_normals && vertices.len() > 20_000 {
        return Err("includeNormals is limited to meshes of at most 20,000 vertices.".into());
    }
    // Topology uses welded positions so seams split for UVs or normals still connect.
    let mut canonical = vec![0usize; vertices.len()];
    let mut seen: HashMap<[i64; 3], usize> = HashMap::new();
    for (i, p) in vertices.iter().enumerate() {
        let key = if weld > 0.0 {
            p.map(|c| (c / weld).round() as i64)
        } else {
            p.map(|c| (c + 0.0).to_bits() as i64)
        };
        canonical[i] = *seen.entry(key).or_insert(i);
    }
    let duplicate_vertices = vertices.len() - seen.len();
    let mut referenced = vec![false; vertices.len()];
    let mut area = 0.0;
    let mut volume6 = 0.0;
    let mut weighted = [0.0; 3];
    let mut area_centroid = [0.0; 3];
    let mut degenerate = Vec::new();
    let mut degenerate_count = 0usize;
    let mut faces: HashSet<[usize; 3]> = HashSet::new();
    let mut duplicate_triangles = 0usize;
    let mut edges: HashMap<(usize, usize), (u32, i32)> = HashMap::new();
    let mut union = UnionFind((0..vertices.len()).collect());
    let mut vertex_normals = vec![[0.0; 3]; if include_normals { vertices.len() } else { 0 }];
    let mut face_normals = Vec::new();
    let eps2 = tolerance(&vertices);
    let mut topology_faces = 0usize;
    for (t, tri) in triangles.iter().enumerate() {
        for &v in tri {
            referenced[v] = true;
        }
        let [a, b, c] = tri.map(|v| vertices[v]);
        let n = cross(sub(b, a), sub(c, a));
        let face_area = norm2(n).sqrt() / 2.0;
        let ids = tri.map(|v| canonical[v]);
        if include_normals {
            face_normals.push(if face_area > 0.0 {
                checked_array(scale(n, 0.5 / face_area).to_vec())?
            } else {
                Value::Null
            });
        }
        // eps2 is (1e-9 * diagonal)^2, so this flags faces below 1e-14 of diagonal^2.
        if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] || face_area <= eps2 * 1e4 {
            degenerate_count += 1;
            if degenerate.len() < MAX_REPORTED {
                degenerate.push(t);
            }
            continue;
        }
        area += face_area;
        let v6 = dot(a, cross(b, c));
        volume6 += v6;
        weighted = add(weighted, scale(add(add(a, b), c), v6 / 4.0));
        area_centroid = add(area_centroid, scale(add(add(a, b), c), face_area / 3.0));
        let mut key = ids;
        key.sort_unstable();
        if !faces.insert(key) {
            duplicate_triangles += 1;
        }
        topology_faces += 1;
        for (x, y) in [(ids[0], ids[1]), (ids[1], ids[2]), (ids[2], ids[0])] {
            let entry = edges.entry((x.min(y), x.max(y))).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += if x < y { 1 } else { -1 };
            union.union(x, y);
        }
        if include_normals {
            for &v in tri {
                vertex_normals[canonical[v]] = add(vertex_normals[canonical[v]], n);
            }
        }
    }
    let mut boundary = Vec::new();
    let (mut boundary_count, mut non_manifold, mut inconsistent) = (0usize, 0usize, 0usize);
    for (&(a, b), &(count, balance)) in &edges {
        match count {
            1 => {
                boundary_count += 1;
                if boundary.len() < MAX_REPORTED {
                    boundary.push([a, b]);
                }
            }
            2 if balance != 0 => inconsistent += 1,
            2 => {}
            _ => non_manifold += 1,
        }
    }
    boundary.sort_unstable();
    let used: HashSet<usize> = edges.keys().flat_map(|&(a, b)| [a, b]).collect();
    let components: HashSet<usize> = used.iter().map(|&v| union.find(v)).collect();
    let euler = used.len() as i64 - edges.len() as i64 + topology_faces as i64;
    let watertight = topology_faces > 0 && boundary_count == 0 && non_manifold == 0;
    let closed_volume = volume6 / 6.0;
    // The signed volume is meaningful only for a closed, consistently wound surface.
    let solid = watertight && inconsistent == 0;
    let centroid = if solid && closed_volume.abs() > 0.0 {
        scale(weighted, 1.0 / volume6)
    } else if area > 0.0 {
        scale(area_centroid, 1.0 / area)
    } else {
        [f64::NAN; 3]
    };
    let referenced_points: Vec<P3> = (0..vertices.len())
        .filter(|&v| referenced[v])
        .map(|v| vertices[v])
        .collect();
    let mut result = json!({
        "vertexCount":vertices.len(),"triangleCount":triangles.len(),
        "uniquePositions":seen.len(),"duplicateVertices":duplicate_vertices,
        "unreferencedVertices":referenced.iter().filter(|r| !**r).count(),
        "surfaceArea":area,"volume":if solid { json!(closed_volume.abs()) } else { Value::Null },
        "signedVolume":closed_volume,
        "centroid":if centroid[0].is_finite() { checked_array(centroid.to_vec())? } else { Value::Null },
        "centroidKind":if solid && closed_volume != 0.0 { "volume" } else { "surface" },
        "degenerateTriangles":degenerate_count,"degenerateTriangleIndices":degenerate,
        "duplicateTriangles":duplicate_triangles,"edgeCount":edges.len(),
        "boundaryEdges":boundary_count,"boundaryEdgeSample":boundary,
        "nonManifoldEdges":non_manifold,"inconsistentlyWoundEdges":inconsistent,
        "watertight":watertight,"consistentWinding":inconsistent == 0,
        "outwardFacing":solid.then_some(closed_volume > 0.0),
        "components":components.len(),"eulerCharacteristic":euler,
        "genus":(watertight && components.len() == 1 && euler <= 2 && euler % 2 == 0).then_some((2 - euler) / 2),
    });
    if !referenced_points.is_empty() {
        let (lo, hi) = bounds(&referenced_points);
        result["bounds"] =
            json!({"min":checked_array(lo.to_vec())?,"max":checked_array(hi.to_vec())?});
    }
    if include_normals {
        let normals = (0..vertices.len())
            .map(|v| {
                let n = vertex_normals[canonical[v]];
                let length = norm2(n).sqrt();
                if length > 0.0 {
                    checked_array(scale(n, 1.0 / length).to_vec())
                } else {
                    Ok(Value::Null)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        result["faceNormals"] = json!(face_normals);
        result["vertexNormals"] = json!(normals);
    }
    Ok(result)
}

fn cross2(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn hull_2d(points: &[[f64; 2]]) -> Result<Value, String> {
    let mut order: Vec<usize> = (0..points.len()).collect();
    order.sort_by(|&a, &b| {
        points[a][0]
            .total_cmp(&points[b][0])
            .then(points[a][1].total_cmp(&points[b][1]))
    });
    order.dedup_by(|a, b| points[*a] == points[*b]);
    let extent = points
        .iter()
        .flat_map(|p| p.iter())
        .fold(0.0f64, |m, v| m.max(v.abs()))
        .max(1e-300);
    let eps = extent * extent * 1e-12;
    let mut hull: Vec<usize> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let sequence: Box<dyn Iterator<Item = &usize>> = if pass == 0 {
            Box::new(order.iter())
        } else {
            Box::new(order.iter().rev())
        };
        for &i in sequence {
            while hull.len() >= start + 2
                && cross2(
                    points[hull[hull.len() - 2]],
                    points[hull[hull.len() - 1]],
                    points[i],
                ) <= eps
            {
                hull.pop();
            }
            hull.push(i);
        }
        hull.pop();
    }
    if hull.len() < 3 {
        let ends: Vec<usize> = if order.len() == 1 {
            vec![order[0]]
        } else {
            vec![order[0], *order.last().unwrap()]
        };
        return Ok(
            json!({"dimensions":2,"degenerate":if ends.len() == 1 {"point"} else {"segment"},
            "vertices":ends,"area":0.0}),
        );
    }
    let mut area = 0.0;
    let mut perimeter = 0.0;
    for i in 0..hull.len() {
        let (a, b) = (points[hull[i]], points[hull[(i + 1) % hull.len()]]);
        area += a[0] * b[1] - a[1] * b[0];
        perimeter += ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    }
    Ok(
        json!({"dimensions":2,"degenerate":null,"vertices":hull,"vertexCount":hull.len(),
        "winding":"ccw","area":area / 2.0,"perimeter":perimeter}),
    )
}

struct Face {
    v: [usize; 3],
    normal: P3,
    offset: f64,
    alive: bool,
}

fn make_face(points: &[P3], v: [usize; 3]) -> Face {
    let normal = cross(
        sub(points[v[1]], points[v[0]]),
        sub(points[v[2]], points[v[0]]),
    );
    let length = norm2(normal).sqrt();
    let normal = if length > 0.0 {
        scale(normal, 1.0 / length)
    } else {
        normal
    };
    Face {
        v,
        normal,
        offset: dot(normal, points[v[0]]),
        alive: true,
    }
}

/// Incremental 3D convex hull; returns outward-facing CCW triangles.
fn hull_faces(points: &[P3]) -> Result<Vec<[usize; 3]>, String> {
    let (lo, hi) = bounds(points);
    let eps = norm2(sub(hi, lo)).sqrt().max(1e-300) * 1e-10;
    // Initial tetrahedron from extreme points.
    let i0 = (0..points.len())
        .min_by(|&a, &b| points[a][0].total_cmp(&points[b][0]))
        .unwrap();
    let i1 = (0..points.len())
        .max_by(|&a, &b| {
            norm2(sub(points[a], points[i0])).total_cmp(&norm2(sub(points[b], points[i0])))
        })
        .unwrap();
    let line = sub(points[i1], points[i0]);
    if norm2(line).sqrt() <= eps {
        return Err("All points coincide; the hull is a single point.".into());
    }
    let line_distance = |p: P3| norm2(cross(sub(p, points[i0]), line)).sqrt() / norm2(line).sqrt();
    let i2 = (0..points.len())
        .max_by(|&a, &b| line_distance(points[a]).total_cmp(&line_distance(points[b])))
        .unwrap();
    if line_distance(points[i2]) <= eps {
        return Err("Points are collinear; there is no 3D hull.".into());
    }
    let base = make_face(points, [i0, i1, i2]);
    let plane = |p: P3| dot(base.normal, p) - base.offset;
    let i3 = (0..points.len())
        .max_by(|&a, &b| plane(points[a]).abs().total_cmp(&plane(points[b]).abs()))
        .unwrap();
    if plane(points[i3]).abs() <= eps {
        return Err("Points are coplanar; pass 2D points for a planar hull.".into());
    }
    let inside = scale(
        add(add(points[i0], points[i1]), add(points[i2], points[i3])),
        0.25,
    );
    let mut faces: Vec<Face> = Vec::new();
    for v in [[i0, i1, i2], [i0, i1, i3], [i0, i2, i3], [i1, i2, i3]] {
        let mut face = make_face(points, v);
        if dot(face.normal, inside) - face.offset > 0.0 {
            face = make_face(points, [v[0], v[2], v[1]]);
        }
        faces.push(face);
    }
    for (p, &point) in points.iter().enumerate() {
        if [i0, i1, i2, i3].contains(&p) {
            continue;
        }
        let visible: Vec<usize> = (0..faces.len())
            .filter(|&f| faces[f].alive && dot(faces[f].normal, point) - faces[f].offset > eps)
            .collect();
        if visible.is_empty() {
            continue;
        }
        let mut directed: HashSet<(usize, usize)> = HashSet::new();
        for &f in &visible {
            let v = faces[f].v;
            for e in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
                directed.insert(e);
            }
            faces[f].alive = false;
        }
        let horizon: Vec<(usize, usize)> = directed
            .iter()
            .copied()
            .filter(|&(a, b)| !directed.contains(&(b, a)))
            .collect();
        for (a, b) in horizon {
            faces.push(make_face(points, [a, b, p]));
        }
        if faces.len() > 64 * MAX_HULL_3D_POINTS {
            faces.retain(|f| f.alive);
        }
    }
    Ok(faces.into_iter().filter(|f| f.alive).map(|f| f.v).collect())
}

fn hull_3d(points: &[P3]) -> Result<Value, String> {
    let faces = hull_faces(points)?;
    let mut vertices: Vec<usize> = faces.iter().flatten().copied().collect();
    vertices.sort_unstable();
    vertices.dedup();
    let mut area = 0.0;
    let mut volume6 = 0.0;
    for face in &faces {
        let [a, b, c] = face.map(|v| points[v]);
        area += norm2(cross(sub(b, a), sub(c, a))).sqrt() / 2.0;
        volume6 += dot(a, cross(b, c));
    }
    Ok(
        json!({"dimensions":3,"degenerate":null,"vertices":vertices,"vertexCount":vertices.len(),
        "faceCount":faces.len(),"faces":faces,
        "winding":"ccw_outward","surfaceArea":area,"volume":volume6 / 6.0}),
    )
}

fn convex_hull(args: &Value) -> Result<Value, String> {
    let rows = input::property(args, "points")?
        .as_array()
        .ok_or("'points' must be an array.")?;
    let dimensions = rows
        .first()
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or("'points' must contain at least one point.")?;
    match dimensions {
        2 => {
            let points: Vec<[f64; 2]> = input::points(args, "points", 2, 1, MAX_POINTS)?
                .into_iter()
                .map(|p| [p[0], p[1]])
                .collect();
            hull_2d(&points)
        }
        3 => hull_3d(&points3(args, "points", MAX_HULL_3D_POINTS)?),
        _ => Err("Points must all be 2D or all be 3D.".into()),
    }
}

fn signed_area(points: &[[f64; 2]], ring: &[usize]) -> f64 {
    (0..ring.len())
        .map(|i| {
            let (a, b) = (points[ring[i]], points[ring[(i + 1) % ring.len()]]);
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
        / 2.0
}

fn in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    cross2(a, b, p) >= 0.0 && cross2(b, c, p) >= 0.0 && cross2(c, a, p) >= 0.0
}

/// Joins a clockwise hole into the counter-clockwise outer ring with a bridge from the
/// hole's rightmost vertex to a mutually visible ring vertex (Eberly's method).
fn bridge(points: &[[f64; 2]], ring: &mut Vec<usize>, hole: &[usize]) -> Result<(), String> {
    let (hole_start, &m) = hole
        .iter()
        .enumerate()
        .max_by(|a, b| points[*a.1][0].total_cmp(&points[*b.1][0]))
        .unwrap();
    let mp = points[m];
    let mut best: Option<(f64, usize)> = None; // (intersection x, ring position of visible vertex)
    for i in 0..ring.len() {
        let (a, b) = (points[ring[i]], points[ring[(i + 1) % ring.len()]]);
        if (a[1] - mp[1]) * (b[1] - mp[1]) > 0.0 || a[1] == b[1] {
            continue;
        }
        let x = a[0] + (mp[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
        if x < mp[0] || best.is_some_and(|(bx, _)| x >= bx) {
            continue;
        }
        let candidate = if a[0] > b[0] { i } else { (i + 1) % ring.len() };
        best = Some((x, candidate));
    }
    let (x, mut position) = best.ok_or("A hole is not inside the outer polygon.")?;
    let intersection = [x, mp[1]];
    let p = points[ring[position]];
    if p != intersection {
        // A reflex vertex inside triangle (M, I, P) may block the bridge; take the one
        // with the smallest angle to the ray.
        let (t1, t2) = if p[1] < mp[1] {
            (intersection, p)
        } else {
            (p, intersection)
        };
        let mut best_angle = f64::INFINITY;
        for i in 0..ring.len() {
            let r = points[ring[i]];
            let previous = points[ring[(i + ring.len() - 1) % ring.len()]];
            let next = points[ring[(i + 1) % ring.len()]];
            if i == position || cross2(previous, r, next) > 0.0 || r == p {
                continue;
            }
            if in_triangle(r, mp, t1, t2) || in_triangle(r, mp, t2, t1) {
                let angle = (r[1] - mp[1]).abs().atan2(r[0] - mp[0]);
                if angle < best_angle {
                    best_angle = angle;
                    position = i;
                }
            }
        }
    }
    let mut merged = ring[..=position].to_vec();
    merged.extend(hole[hole_start..].iter().chain(&hole[..=hole_start]));
    merged.extend_from_slice(&ring[position..]);
    *ring = merged;
    Ok(())
}

fn polygon_triangulate(args: &Value) -> Result<Value, String> {
    let outer: Vec<[f64; 2]> = input::points(args, "points", 2, 3, MAX_POLYGON_VERTICES)?
        .into_iter()
        .map(|p| [p[0], p[1]])
        .collect();
    let mut points = outer.clone();
    let mut holes: Vec<Vec<usize>> = Vec::new();
    if let Some(raw) = args.get("holes") {
        let list = raw
            .as_array()
            .ok_or("'holes' must be an array of polygons.")?;
        for (h, hole) in list.iter().enumerate() {
            let hole_points =
                input::points(&json!({"hole":hole}), "hole", 2, 3, MAX_POLYGON_VERTICES)
                    .map_err(|e| format!("holes[{h}]: {e}"))?;
            let start = points.len();
            points.extend(hole_points.into_iter().map(|p| [p[0], p[1]]));
            holes.push((start..points.len()).collect());
            if points.len() > MAX_POLYGON_VERTICES {
                return Err(format!("At most {MAX_POLYGON_VERTICES} vertices in total."));
            }
        }
    }
    let mut ring: Vec<usize> = (0..outer.len()).collect();
    let outer_area = signed_area(&points, &ring);
    if outer_area.abs() <= f64::EPSILON {
        return Err("The polygon has zero area.".into());
    }
    if outer_area < 0.0 {
        ring.reverse();
    }
    let mut hole_area = 0.0;
    for hole in &mut holes {
        let area = signed_area(&points, hole);
        hole_area += area.abs();
        if area > 0.0 {
            hole.reverse();
        }
    }
    holes.sort_by(|a, b| {
        let max_x = |h: &Vec<usize>| {
            h.iter()
                .map(|&i| points[i][0])
                .fold(f64::NEG_INFINITY, f64::max)
        };
        max_x(b).total_cmp(&max_x(a))
    });
    for hole in &holes {
        bridge(&points, &mut ring, hole)?;
    }
    // Ear clipping; only reflex vertices can lie inside a candidate ear.
    let mut triangles: Vec<[usize; 3]> = Vec::with_capacity(ring.len());
    let mut stalls = 0;
    let mut i = 0;
    while ring.len() > 3 {
        let n = ring.len();
        let (pi, ci, ni) = ((i + n - 1) % n, i % n, (i + 1) % n);
        let (a, b, c) = (points[ring[pi]], points[ring[ci]], points[ring[ni]]);
        let convex = cross2(a, b, c) > 0.0;
        let is_ear = convex
            && (0..n).all(|j| {
                if j == pi || j == ci || j == ni {
                    return true;
                }
                let p = points[ring[j]];
                if p == a || p == b || p == c {
                    return true;
                }
                let before = points[ring[(j + n - 1) % n]];
                let after = points[ring[(j + 1) % n]];
                cross2(before, p, after) > 0.0 || !in_triangle(p, a, b, c)
            });
        if is_ear {
            triangles.push([ring[pi], ring[ci], ring[ni]]);
            ring.remove(ci);
            stalls = 0;
            i = if ci == 0 { 0 } else { ci - 1 };
        } else {
            i = (i + 1) % n;
            stalls += 1;
            if stalls > n {
                // Remove a collinear or zero-area vertex if one exists; otherwise the
                // polygon self-intersects.
                let flat = (0..n).find(|&k| {
                    cross2(
                        points[ring[(k + n - 1) % n]],
                        points[ring[k]],
                        points[ring[(k + 1) % n]],
                    )
                    .abs()
                        <= f64::EPSILON
                });
                match flat {
                    Some(k) => {
                        ring.remove(k);
                        stalls = 0;
                    }
                    None => return Err(
                        "The polygon is not simple (edges cross) or a hole overlaps the boundary."
                            .into(),
                    ),
                }
            }
        }
    }
    triangles.push([ring[0], ring[1], ring[2]]);
    triangles.retain(|t| cross2(points[t[0]], points[t[1]], points[t[2]]).abs() > 0.0);
    let triangulated: f64 = triangles
        .iter()
        .map(|t| cross2(points[t[0]], points[t[1]], points[t[2]]) / 2.0)
        .sum();
    Ok(
        json!({"triangles":triangles,"triangleCount":triangles.len(),"vertexCount":points.len(),
        "winding":"ccw","area":outer_area.abs() - hole_area,"triangulatedArea":triangulated,
        "holes":holes.len()}),
    )
}
