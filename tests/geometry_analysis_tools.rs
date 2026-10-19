mod common;

use common::{call, response};
use serde_json::{Value, json};

fn close(value: &Value, expected: f64) -> bool {
    (value.as_f64().unwrap() - expected).abs() <= 1e-9 * expected.abs().max(1.0)
}

fn unit_cube() -> (Value, Value) {
    let vertices = json!([
        [0, 0, 0],
        [1, 0, 0],
        [1, 1, 0],
        [0, 1, 0],
        [0, 0, 1],
        [1, 0, 1],
        [1, 1, 1],
        [0, 1, 1]
    ]);
    let faces = json!([
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6,
        3, 0, 4, 3, 4, 7
    ]);
    (vertices, faces)
}

#[test]
fn bounding_volumes_of_a_box_and_its_corners() {
    let corners = json!([
        [-1, -2, -3],
        [1, -2, -3],
        [1, 2, -3],
        [-1, 2, -3],
        [-1, -2, 3],
        [1, -2, 3],
        [1, 2, 3],
        [-1, 2, 3]
    ]);
    let result = call("bounding_volume", json!({"points":corners}));
    assert_eq!(result["aabb"]["min"], json!([-1.0, -2.0, -3.0]));
    assert!(close(&result["aabb"]["volume"], 48.0));
    // The minimum sphere through the box corners has the half-diagonal as radius.
    assert!(close(&result["sphere"]["radius"], 14f64.sqrt()));
    assert!(close(&result["obb"]["volume"], 48.0));
    // Two points: the sphere is centred between them.
    let pair = call(
        "bounding_volume",
        json!({"points":[[0,0,0],[4,0,0]],"kind":"sphere"}),
    );
    assert_eq!(pair["sphere"]["center"], json!([2.0, 0.0, 0.0]));
    assert!(close(&pair["sphere"]["radius"], 2.0));
    assert!(pair.get("aabb").is_none());
}

#[test]
fn obb_recovers_a_rotated_box() {
    // A 4x2x1 box rotated 30 degrees about Z.
    let (s, c) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
    let mut points = Vec::new();
    for x in [-2.0, 2.0] {
        for y in [-1.0, 1.0] {
            for z in [-0.5, 0.5] {
                points.push(json!([c * x - s * y, s * x + c * y, z]));
            }
        }
    }
    let obb = &call("bounding_volume", json!({"points":points,"kind":"obb"}))["obb"];
    assert!(close(&obb["volume"], 8.0), "{obb}");
    let mut half: Vec<f64> = obb["halfExtents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    half.sort_by(f64::total_cmp);
    assert!(
        (half[0] - 0.5).abs() < 1e-9
            && (half[1] - 1.0).abs() < 1e-9
            && (half[2] - 2.0).abs() < 1e-9
    );
}

#[test]
fn mesh_inspect_reports_solid_and_broken_meshes() {
    let (vertices, faces) = unit_cube();
    let cube = call("mesh_inspect", json!({"vertices":vertices,"indices":faces}));
    assert_eq!(cube["watertight"], true);
    assert_eq!(cube["outwardFacing"], true);
    assert_eq!(cube["genus"], 0);
    assert_eq!(cube["eulerCharacteristic"], 2);
    assert!(close(&cube["surfaceArea"], 6.0));
    assert!(close(&cube["volume"], 1.0));
    assert_eq!(cube["centroid"], json!([0.5, 0.5, 0.5]));

    // Drop the top two triangles: an open box with a 4-edge hole.
    let open: Vec<Value> = faces.as_array().unwrap()[..6]
        .iter()
        .chain(&faces.as_array().unwrap()[12..])
        .cloned()
        .collect();
    let open_box = call("mesh_inspect", json!({"vertices":vertices,"indices":open}));
    assert_eq!(
        (
            open_box["watertight"].as_bool(),
            open_box["boundaryEdges"].as_u64()
        ),
        (Some(false), Some(4))
    );
    assert_eq!(open_box["volume"], Value::Null);

    // Flip one triangle: still closed, but the winding is inconsistent.
    let mut flipped = faces.as_array().unwrap().clone();
    flipped.swap(1, 2);
    let bad = call(
        "mesh_inspect",
        json!({"vertices":vertices,"indices":flipped}),
    );
    assert_eq!(
        (
            bad["watertight"].as_bool(),
            bad["consistentWinding"].as_bool()
        ),
        (Some(true), Some(false))
    );
    assert_eq!(bad["inconsistentlyWoundEdges"], 3);
    assert_eq!(bad["volume"], Value::Null);
}

#[test]
fn mesh_inspect_welds_seams_and_flags_defects() {
    // Triangle soup of the cube: 36 vertices, every corner repeated.
    let (vertices, faces) = unit_cube();
    let soup: Vec<Value> = faces
        .as_array()
        .unwrap()
        .iter()
        .map(|i| vertices[i.as_u64().unwrap() as usize].clone())
        .collect();
    let welded = call("mesh_inspect", json!({"vertices":soup}));
    assert_eq!(welded["duplicateVertices"], 28);
    assert_eq!(welded["watertight"], true);
    let mut defective = faces.as_array().unwrap().clone();
    defective.extend([json!(0), json!(0), json!(1)]); // degenerate
    defective.extend(faces.as_array().unwrap()[..3].iter().cloned()); // duplicate
    let result = call(
        "mesh_inspect",
        json!({"vertices":vertices,"indices":defective,"includeNormals":true}),
    );
    assert_eq!(result["degenerateTriangles"], 1);
    assert_eq!(result["degenerateTriangleIndices"], json!([12]));
    assert_eq!(result["duplicateTriangles"], 1);
    assert_eq!(result["nonManifoldEdges"], 3);
    assert_eq!(result["faceNormals"][0], json!([0.0, 0.0, -1.0]));
    assert_eq!(
        response(
            "mesh_inspect",
            json!({"vertices":vertices,"indices":[0,1,9]})
        )["result"]["isError"],
        true
    );
}

#[test]
fn convex_hulls_in_two_and_three_dimensions() {
    let square = call(
        "convex_hull",
        json!({"points":[[0,0],[2,0],[1,1],[2,2],[0,2],[1,0]]}),
    );
    assert_eq!(square["vertices"], json!([0, 1, 3, 4]));
    assert!(close(&square["area"], 4.0));
    assert!(close(&square["perimeter"], 8.0));
    let (vertices, _) = unit_cube();
    let mut points = vertices.as_array().unwrap().clone();
    points.push(json!([0.5, 0.5, 0.5]));
    let cube = call("convex_hull", json!({"points":points}));
    assert_eq!(cube["vertices"], json!([0, 1, 2, 3, 4, 5, 6, 7]));
    assert_eq!(cube["faceCount"], 12);
    assert!(close(&cube["volume"], 1.0));
    assert!(close(&cube["surfaceArea"], 6.0));
    let segment = call("convex_hull", json!({"points":[[0,0],[1,1],[3,3]]}));
    assert_eq!(segment["degenerate"], "segment");
    let flat = response(
        "convex_hull",
        json!({"points":[[0,0,0],[1,0,0],[0,1,0],[1,1,0]]}),
    );
    assert_eq!(flat["result"]["isError"], true);
}

#[test]
fn triangulates_polygons_with_holes() {
    let l_shape = call(
        "polygon_triangulate",
        json!({"points":[[0,0],[2,0],[2,1],[1,1],[1,2],[0,2]]}),
    );
    assert_eq!(l_shape["triangleCount"], 4);
    assert!(close(&l_shape["triangulatedArea"], 3.0));
    let framed = call(
        "polygon_triangulate",
        json!({"points":[[0,0],[10,0],[10,10],[0,10]],"holes":[[[3,3],[7,3],[7,7],[3,7]]]}),
    );
    assert_eq!(framed["triangleCount"], 8);
    assert!(close(&framed["area"], 84.0));
    assert!(close(&framed["triangulatedArea"], 84.0));
    let bow_tie = response(
        "polygon_triangulate",
        json!({"points":[[0,0],[2,2],[2,0],[0,2]]}),
    );
    assert_eq!(bow_tie["result"]["isError"], true);
}

#[test]
fn evaluates_expressions() {
    let eval = |expression: &str, variables: Value| {
        call(
            "expression_evaluate",
            json!({"expression":expression,"variables":variables}),
        )["value"]
            .clone()
    };
    assert_eq!(eval("2 + 3 * 4", json!({})), 14.0);
    assert_eq!(eval("-2^2", json!({})), -4.0);
    assert_eq!(eval("2^3^2", json!({})), 512.0);
    assert_eq!(eval("sqrt(x^2 + y^2)", json!({"x":3,"y":4})), 5.0);
    assert_eq!(eval("x > 2 ? 10 * x : -x", json!({"x":3})), 30.0);
    assert_eq!(
        eval("log(8, 2) + hypot(3, 4) + clamp(15, 0, 10)", json!({})),
        18.0
    );
    let degrees = call(
        "expression_evaluate",
        json!({"expression":"sin(30)","angleUnit":"degrees"}),
    );
    assert!(close(&degrees["value"], 0.5));
    let sweep = call(
        "expression_evaluate",
        json!({"expression":"x^2 - 1","sweepVariable":"x","sweepValues":[0,1,2,3]}),
    );
    assert_eq!(sweep["values"], json!([-1.0, 0.0, 3.0, 8.0]));
    for bad in ["2 +", "foo(1)", "(1", "1 2", "z + 1", "sqrt(-1)", "1/0"] {
        assert_eq!(
            response("expression_evaluate", json!({"expression":bad}))["result"]["isError"],
            true,
            "{bad}"
        );
    }
}

#[test]
fn fits_curves() {
    let line = call(
        "curve_fit",
        json!({"x":[1,2,3,4],"y":[3,5,7,9],"model":"linear","predict":[10]}),
    );
    assert!(close(&line["parameters"]["a"], 1.0) && close(&line["parameters"]["b"], 2.0));
    assert!(close(&line["r2"], 1.0));
    assert!(close(&line["predictions"][0], 21.0));
    let x: Vec<f64> = (0..20).map(|i| i as f64 * 0.5 - 3.0).collect();
    let y: Vec<f64> = x.iter().map(|x| 1.0 - 2.0 * x + 0.5 * x * x * x).collect();
    let cubic = call(
        "curve_fit",
        json!({"x":x,"y":y,"model":"polynomial","degree":3}),
    );
    let c: Vec<f64> = cubic["coefficients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    for (got, want) in c.iter().zip([1.0, -2.0, 0.0, 0.5]) {
        assert!((got - want).abs() < 1e-9, "{c:?}");
    }
    let xs: Vec<f64> = (1..=15).map(f64::from).collect();
    let exponential: Vec<f64> = xs.iter().map(|x| 2.5 * (0.3 * x).exp()).collect();
    let fit = call(
        "curve_fit",
        json!({"x":xs,"y":exponential,"model":"exponential"}),
    );
    assert!((fit["parameters"]["a"].as_f64().unwrap() - 2.5).abs() < 1e-9);
    assert!((fit["parameters"]["b"].as_f64().unwrap() - 0.3).abs() < 1e-12);
    let power: Vec<f64> = xs.iter().map(|x| 1.7 * x.powf(1.4)).collect();
    let fit = call("curve_fit", json!({"x":xs,"y":power,"model":"power"}));
    assert!((fit["parameters"]["b"].as_f64().unwrap() - 1.4).abs() < 1e-12);
    let logarithmic: Vec<f64> = xs.iter().map(|x| 3.0 + 2.0 * x.ln()).collect();
    let fit = call(
        "curve_fit",
        json!({"x":xs,"y":logarithmic,"model":"logarithmic"}),
    );
    assert!(close(&fit["parameters"]["a"], 3.0) && close(&fit["parameters"]["b"], 2.0));
    for bad in [
        json!({"x":[1],"y":[1],"model":"linear"}),
        json!({"x":[1,2],"y":[1,-1],"model":"exponential"}),
        json!({"x":[1,2,3],"y":[1,2,3],"model":"polynomial","degree":3}),
        json!({"x":[1,2],"y":[1,2],"model":"spline"}),
    ] {
        assert_eq!(response("curve_fit", bad)["result"]["isError"], true);
    }
}
