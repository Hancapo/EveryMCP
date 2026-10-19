use serde_json::{Map, Value, json};

pub fn list() -> Vec<Value> {
    let mut tools = Vec::new();
    add(
        &mut tools,
        "add",
        "Add two numbers.",
        &[("firstNumber", "number"), ("secondNumber", "number")],
    );
    add(
        &mut tools,
        "subtract",
        "Subtract subtrahend from minuend.",
        &[("minuend", "number"), ("subtrahend", "number")],
    );
    add(
        &mut tools,
        "multiply",
        "Multiply two numbers.",
        &[("firstNumber", "number"), ("secondNumber", "number")],
    );
    add(
        &mut tools,
        "division",
        "Divide numerator by denominator.",
        &[("numerator", "number"), ("denominator", "number")],
    );
    add(
        &mut tools,
        "sum",
        "Sum a nonempty array of numbers.",
        &[("numbers", "number[]")],
    );
    add(
        &mut tools,
        "modulo",
        "Remainder of numerator divided by denominator.",
        &[("numerator", "number"), ("denominator", "number")],
    );
    for name in [
        "floor",
        "ceiling",
        "round",
        "sin",
        "arcsin",
        "cos",
        "arccos",
        "tan",
        "arctan",
        "radiansToDegrees",
        "degreesToRadians",
    ] {
        add(
            &mut tools,
            name,
            &format!(
                "Calculate {name} for a number; angles are radians except in explicit conversions."
            ),
            &[("number", "number")],
        );
    }
    for name in ["mean", "median", "mode", "min", "max"] {
        add(
            &mut tools,
            name,
            &format!("Calculate {name} of a nonempty array of numbers."),
            &[("numbers", "number[]")],
        );
    }
    add(
        &mut tools,
        "integer_convert",
        "Interpret a signed/unsigned integer in a fixed width and show decimal, hexadecimal, binary and endian bytes. Integer input is a string to preserve 64-bit precision.",
        &[("value", "integer"), ("bits", "integerNumber")],
    );
    add(
        &mut tools,
        "bitfield_extract",
        "Extract a bitfield from a 64-bit integer; offset is the least-significant bit index.",
        &[
            ("value", "integer"),
            ("offset", "integerNumber"),
            ("width", "integerNumber"),
        ],
    );
    add(
        &mut tools,
        "bitfield_insert",
        "Replace a bitfield in an 8/16/32/64-bit word.",
        &[
            ("value", "integer"),
            ("field", "integer"),
            ("offset", "integerNumber"),
            ("width", "integerNumber"),
            ("bits", "integerNumber"),
        ],
    );
    add(
        &mut tools,
        "address_translate",
        "Convert an image virtual address to RVA and to a runtime virtual address after rebasing.",
        &[
            ("address", "integer"),
            ("imageBase", "integer"),
            ("runtimeBase", "integer"),
        ],
    );
    add(
        &mut tools,
        "align_address",
        "Align a 64-bit address up and down to a power-of-two boundary; report bytes of upward padding.",
        &[("address", "integer"), ("alignment", "integer")],
    );
    add(
        &mut tools,
        "relative_target",
        "Resolve a signed PC-relative displacement from the end of an instruction.",
        &[
            ("instructionAddress", "integer"),
            ("instructionSize", "integerNumber"),
            ("displacement", "integer"),
        ],
    );
    add(
        &mut tools,
        "ieee754_decode",
        "Decode a 32- or 64-bit IEEE-754 bit pattern; non-finite values are returned as strings.",
        &[("bits", "integerNumber"), ("pattern", "integer")],
    );
    add(
        &mut tools,
        "vector_calculate",
        "Vector dot, cross, length, normalize, distance or angle (radians). Vectors have 2 or 3 components; cross requires 3.",
        &[("operation", "string"), ("a", "number[]")],
    );
    add(
        &mut tools,
        "matrix_transform",
        "Transform a 3D point or direction by a row-major 4x4 matrix using column vectors. Translation is in indices 3, 7 and 11.",
        &[
            ("matrix", "number[]"),
            ("vector", "number[]"),
            ("kind", "string"),
        ],
    );
    add(
        &mut tools,
        "matrix_multiply",
        "Compose two row-major 4x4 matrices as a × b, applying b first to column vectors.",
        &[("a", "number[]"), ("b", "number[]")],
    );
    add(
        &mut tools,
        "matrix_inverse",
        "Invert a row-major 4x4 matrix; rejects singular matrices.",
        &[("matrix", "number[]")],
    );
    add(
        &mut tools,
        "quaternion_rotate",
        "Rotate a 3D vector by a quaternion [x,y,z,w]; quaternion is normalized first.",
        &[("quaternion", "number[]"), ("vector", "number[]")],
    );
    add(
        &mut tools,
        "barycentric_2d",
        "Calculate barycentric weights and inside status for a 2D triangle.",
        &[
            ("point", "number[]"),
            ("a", "number[]"),
            ("b", "number[]"),
            ("c", "number[]"),
        ],
    );
    add(
        &mut tools,
        "ray_triangle_intersect",
        "Intersect a 3D ray with a triangle; returns distance, point and barycentric weights on a hit.",
        &[
            ("origin", "number[]"),
            ("direction", "number[]"),
            ("a", "number[]"),
            ("b", "number[]"),
            ("c", "number[]"),
        ],
    );
    add_extra(
        &mut tools,
        "pe_address_map",
        "Map PE VA/RVA/file offsets using section headers. Modes: rva_to_file, file_to_rva, va_to_file, file_to_va. Integer fields are strings.",
        &[
            ("mode", "string"),
            ("value", "integer"),
            ("imageBase", "integer"),
            ("sizeOfHeaders", "integer"),
            ("sections", "object[]"),
        ],
        &[],
    );
    add_extra(
        &mut tools,
        "binary_unpack",
        "Decode bytes at offset with count and optional stride. Types: u8/i8/u16/i16/u32/i32/u64/i64/f16/f32/f64; endian little or big. Integer results are strings.",
        &[
            ("data", "string"),
            ("type", "string"),
            ("endian", "string"),
            ("offset", "integerNumber"),
            ("count", "integerNumber"),
        ],
        &[("stride", "integerNumber")],
    );
    add_extra(
        &mut tools,
        "binary_pack",
        "Encode integer strings or finite float numbers as bytes. Types: u8/i8/u16/i16/u32/i32/u64/i64/f16/f32/f64; endian little or big.",
        &[
            ("values", "scalar[]"),
            ("type", "string"),
            ("endian", "string"),
        ],
        &[],
    );
    add_extra(
        &mut tools,
        "bitwise_word",
        "Fixed-width and/or/xor/not/shift_left/shift_right/rotate_left/rotate_right. Shifts are logical. Width 8,16,32,64.",
        &[
            ("operation", "string"),
            ("a", "integer"),
            ("bits", "integerNumber"),
        ],
        &[("b", "integer"), ("shift", "integerNumber")],
    );
    add_extra(
        &mut tools,
        "fixed_width_alu",
        "Fixed-width add/subtract/multiply with wrap and carry, signed overflow, zero and sign flags.",
        &[
            ("operation", "string"),
            ("a", "integer"),
            ("b", "integer"),
            ("bits", "integerNumber"),
        ],
        &[],
    );
    add_extra(
        &mut tools,
        "leb128_codec",
        "Encode or decode signed/unsigned LEB128. Decode consumes the first complete value and reports byte count.",
        &[("mode", "string"), ("signed", "boolean")],
        &[("value", "integer"), ("data", "string")],
    );
    add_extra(
        &mut tools,
        "x86_effective_address",
        "Compute base + index*scale + displacement with 16/32/64-bit wrap; scale is 1,2,4,8.",
        &[
            ("base", "integer"),
            ("index", "integer"),
            ("scale", "integerNumber"),
            ("displacement", "integer"),
            ("bits", "integerNumber"),
        ],
        &[],
    );
    add_extra(
        &mut tools,
        "x86_disasm",
        "Disassemble hex machine code (up to 64 KiB) as 16/32/64-bit x86 (bits defaults to 64) at a virtual address. syntax is intel (default), nasm, masm, gas or fast. Returns each instruction's address, bytes, text, mnemonic, flow control and resolved branch or RIP-relative targets; stops after limit instructions (default 1000).",
        &[("code", "string")],
        &[
            ("bits", "integerNumber"),
            ("address", "integer"),
            ("syntax", "string"),
            ("limit", "integerNumber"),
        ],
    );
    add_extra(
        &mut tools,
        "x86_assemble",
        "Re-encode existing hex machine code (not assembly text) from sourceAddress to targetAddress, fixing relative branches, calls and RIP-relative operands so relocated patch bytes still reach the same targets; branches may grow. dontFixBranches keeps them as decoded. Fails on bytes that do not decode as valid instructions.",
        &[("code", "string")],
        &[
            ("bits", "integerNumber"),
            ("sourceAddress", "integer"),
            ("targetAddress", "integer"),
            ("dontFixBranches", "boolean"),
        ],
    );
    add_extra(
        &mut tools,
        "signature_generate",
        "Build the shortest byte signature, starting at one instruction, that is unique in a whole file ('path' up to 256 MiB, addressed by file 'offset' or PE 'rva') or in hex 'code' (up to 64 KiB, from 'offset'). Grows one decoded instruction at a time up to maxBytes (default 128). Branch, call and RIP-relative operand bytes are wildcarded unless exactOperands=true. Returns code-style (??), IDA-style (?) and x/? mask forms, matchCount and whether uniqueness was reached.",
        &[],
        &[
            ("path", "string"),
            ("rva", "integer"),
            ("code", "string"),
            ("offset", "integerNumber"),
            ("maxBytes", "integerNumber"),
            ("bits", "integerNumber"),
            ("address", "integer"),
            ("minInstructions", "integerNumber"),
            ("exactOperands", "boolean"),
        ],
    );
    add_extra(
        &mut tools,
        "struct_layout",
        "Lay out C structs and unions for an ABI (win64 default, win32, sysv64, sysv32): each field's offset, size and alignment (nested members dotted), bit-field storage units and bit offsets, padding gaps and sizeof. Understands typedefs, nested and anonymous structs/unions, enums, function pointers, #define constants in array sizes, #pragma pack, pack, alignas and packed/aligned attributes. name picks the struct (default: the last one defined). Optionally decodes 'data' hex, or bytes read from 'path' at 'offset', through the layout (endian little by default).",
        &[("source", "string")],
        &[
            ("name", "string"),
            ("abi", "string"),
            ("pack", "integerNumber"),
            ("data", "string"),
            ("path", "string"),
            ("offset", "integer"),
            ("endian", "string"),
        ],
    );
    add_extra(
        &mut tools,
        "pe_relocation_apply",
        "Apply PE HIGHLOW (32-bit) or DIR64 (64-bit) base relocation delta, wrapping to field width.",
        &[
            ("value", "integer"),
            ("oldBase", "integer"),
            ("newBase", "integer"),
            ("type", "string"),
        ],
        &[],
    );
    add_extra(
        &mut tools,
        "packed_vertex_decode",
        "Decode packed DXGI-style components from an integer pattern: R10G10B10A2_UNORM, R8G8B8A8_UNORM, R8G8B8A8_SNORM, R16G16_FLOAT.",
        &[("format", "string"), ("pattern", "integer")],
        &[],
    );
    add_extra(
        &mut tools,
        "crc_compute",
        "Bitwise CRC over hex bytes with explicit width, polynomial (normal form), init, xorOut, reflectIn and reflectOut.",
        &[
            ("data", "string"),
            ("width", "integerNumber"),
            ("polynomial", "integer"),
            ("init", "integer"),
            ("xorOut", "integer"),
            ("reflectIn", "boolean"),
            ("reflectOut", "boolean"),
        ],
        &[],
    );
    add_extra(
        &mut tools,
        "hash_compute",
        "Non-cryptographic hashes used by file formats and engines: joaat, fnv1_32, fnv1a_32, fnv1_64, fnv1a_64, murmur3_32, xxh32, xxh64, xxh3_64, djb2, djb2a, sdbm. Hash one 'data' string or up to 1000 'inputs'. encoding is utf8 (default), utf16le or hex; caseFold none/lower/upper folds ASCII letters of text input. seed is an integer string (FNV takes none; djb2 defaults to 5381). Returns hex, unsigned and signed values.",
        &[("algorithm", "string")],
        &[
            ("data", "string"),
            ("inputs", "string[]"),
            ("encoding", "string"),
            ("caseFold", "string"),
            ("seed", "integer"),
        ],
    );

    add_extra(
        &mut tools,
        "compose_transform",
        "Compose row-major 4x4 affine matrix as T*R*S from translation, quaternion XYZW and scale.",
        &[
            ("translation", "number[]"),
            ("rotation", "number[]"),
            ("scale", "number[]"),
        ],
        &[],
    );
    add_extra(
        &mut tools,
        "decompose_transform",
        "Decompose affine row-major 4x4 matrix into translation, quaternion, signed scale and XY/XZ/YZ shear. Reflections choose negative X scale.",
        &[("matrix", "number[]")],
        &[],
    );
    add_extra(
        &mut tools,
        "rotation_convert",
        "Convert quaternion XYZW, axis_angle [x,y,z,angle], Euler XYZ radians or row-major 3x3 matrix. Euler order is XYZ.",
        &[("from", "string"), ("to", "string"), ("value", "number[]")],
        &[("order", "string")],
    );
    add_extra(
        &mut tools,
        "quaternion_slerp",
        "Shortest-arc spherical interpolation between two quaternion XYZW rotations; t in [0,1].",
        &[("a", "number[]"), ("b", "number[]"), ("t", "number")],
        &[],
    );
    add_extra(
        &mut tools,
        "project_unproject",
        "Project/unproject 3D point with row-major world-to-clip matrix, top-left viewport [x,y,width,height], depthRange zero_to_one or minus_one_to_one.",
        &[
            ("mode", "string"),
            ("matrix", "number[]"),
            ("viewport", "number[]"),
            ("vector", "number[]"),
            ("depthRange", "string"),
        ],
        &[],
    );
    add_extra(
        &mut tools,
        "ray_primitive_intersect",
        "Intersect 3D ray with plane, sphere or AABB. Direction is normalized; distance is Euclidean.",
        &[
            ("primitive", "string"),
            ("origin", "number[]"),
            ("direction", "number[]"),
        ],
        &[
            ("normal", "number[]"),
            ("distance", "number"),
            ("center", "number[]"),
            ("radius", "number"),
            ("min", "number[]"),
            ("max", "number[]"),
        ],
    );
    add_extra(
        &mut tools,
        "closest_point",
        "Closest point to a 3D query on segment, triangle or AABB.",
        &[("primitive", "string"), ("point", "number[]")],
        &[
            ("a", "number[]"),
            ("b", "number[]"),
            ("c", "number[]"),
            ("min", "number[]"),
            ("max", "number[]"),
        ],
    );
    add_extra(
        &mut tools,
        "segment_intersect_2d",
        "Intersect two 2D closed segments; returns none, point or overlap.",
        &[
            ("p1", "number[]"),
            ("p2", "number[]"),
            ("q1", "number[]"),
            ("q2", "number[]"),
        ],
        &[],
    );
    add_extra(
        &mut tools,
        "polygon_measure_2d",
        "Signed area, absolute area, centroid and winding of a simple 2D polygon (implicit closure).",
        &[("points", "number[][]")],
        &[],
    );
    add_extra(
        &mut tools,
        "frustum_test",
        "Classify point, sphere or AABB against row-major world-to-clip matrix. depthRange zero_to_one or minus_one_to_one.",
        &[
            ("matrix", "number[]"),
            ("primitive", "string"),
            ("depthRange", "string"),
        ],
        &[
            ("point", "number[]"),
            ("center", "number[]"),
            ("radius", "number"),
            ("min", "number[]"),
            ("max", "number[]"),
        ],
    );

    add_extra(
        &mut tools,
        "power_root_log",
        "pow(a,b), root(a,b), log(a,b base; omitted means natural log), or exp(a).",
        &[("operation", "string"), ("a", "number")],
        &[("b", "number")],
    );
    add_extra(
        &mut tools,
        "integer_power",
        "Raise an arbitrary-size integer base to a nonnegative integer exponent exactly; decimal result is capped at 10000 digits.",
        &[("base", "integer"), ("exponent", "integerNumber")],
        &[],
    );
    add_extra(
        &mut tools,
        "exact_fraction",
        "Exact add/subtract/multiply/divide/normalize/compare over arbitrary-size integer fraction objects.",
        &[("operation", "string"), ("a", "fraction")],
        &[("b", "fraction")],
    );
    add_extra(
        &mut tools,
        "gcd_extended",
        "Exact greatest common divisor, least common multiple and Bezout coefficients for integer strings.",
        &[("a", "integer"), ("b", "integer")],
        &[],
    );
    add_extra(
        &mut tools,
        "modular_arithmetic",
        "Canonical mod add/multiply/pow/inverse with positive arbitrary-size modulus.",
        &[
            ("operation", "string"),
            ("a", "integer"),
            ("modulus", "integer"),
        ],
        &[("b", "integer")],
    );
    add_extra(
        &mut tools,
        "combinatorics_exact",
        "Exact factorial, permutation nPk or combination nCk; n <= 5000.",
        &[("operation", "string"), ("n", "integerNumber")],
        &[("k", "integerNumber")],
    );
    add_extra(
        &mut tools,
        "complex_calculate",
        "Complex add/subtract/multiply/divide/magnitude/argument/from_polar using [real,imag] pairs or [radius,angle] for from_polar.",
        &[("operation", "string"), ("a", "number[]")],
        &[("b", "number[]")],
    );
    add_extra(
        &mut tools,
        "polynomial_evaluate",
        "Evaluate polynomial and derivative at x via Horner; coefficients are highest degree first.",
        &[("coefficients", "number[]"), ("x", "number")],
        &[],
    );
    add_extra(
        &mut tools,
        "linear_system_solve",
        "Solve square Ax=b by pivoted Gaussian elimination; returns solution and max absolute residual. n <= 32.",
        &[("matrix", "number[][]"), ("b", "number[]")],
        &[],
    );
    add_extra(
        &mut tools,
        "descriptive_statistics",
        "Mean, sample/population variance, standard deviation, min/max/median/quartiles; optional Pearson correlation against other.",
        &[("numbers", "number[]")],
        &[("sample", "boolean"), ("other", "number[]")],
    );
    add_extra(
        &mut tools,
        "interpolate_samples",
        "Scalar linear, Hermite or Catmull-Rom interpolation at t in [0,1]. p1,p2 are segment endpoints.",
        &[
            ("mode", "string"),
            ("p1", "number"),
            ("p2", "number"),
            ("t", "number"),
        ],
        &[
            ("p0", "number"),
            ("p3", "number"),
            ("tangent1", "number"),
            ("tangent2", "number"),
        ],
    );
    add_extra(
        &mut tools,
        "bounding_volume",
        "Bounding volumes of up to 100,000 3D points: AABB (min, max, center, halfExtents, volume, surface area), the exact minimum enclosing sphere, and a PCA-fitted OBB (center, right-handed axes, halfExtents, XYZW rotation quaternion, volume). kind is all (default), aabb, sphere or obb.",
        &[("points", "number[][]")],
        &[("kind", "string")],
    );
    add_extra(
        &mut tools,
        "mesh_inspect",
        "Inspect a triangle mesh from 3D vertices and a flat triangle index list (omit indices for a triangle soup): surface area, enclosed volume and centroid, bounds, degenerate and duplicate triangles, duplicate and unreferenced vertices, boundary and non-manifold edges, winding consistency, watertightness, outward orientation, connected components, Euler characteristic and genus. Topology welds equal positions (weldTolerance for near-equal) so UV seams do not count as holes. includeNormals adds face and area-weighted vertex normals (up to 20,000 vertices).",
        &[("vertices", "number[][]")],
        &[
            ("indices", "number[]"),
            ("weldTolerance", "number"),
            ("includeNormals", "boolean"),
        ],
    );
    add_extra(
        &mut tools,
        "convex_hull",
        "Convex hull of 2D points (up to 100,000; returns CCW vertex indices, area, perimeter, or a point/segment for degenerate input) or 3D points (up to 10,000; returns vertex indices, outward CCW triangle faces, surface area and volume).",
        &[("points", "number[][]")],
        &[],
    );
    add_extra(
        &mut tools,
        "polygon_triangulate",
        "Triangulate a simple 2D polygon, optionally with holes, by ear clipping (up to 5,000 vertices in total). Returns CCW triangles as indices into the outer points followed by each hole's points in order, plus polygon and triangulated area for checking.",
        &[("points", "number[][]")],
        &[("holes", "number[][][]")],
    );
    add_extra(
        &mut tools,
        "expression_evaluate",
        "Evaluate one math expression with named variables: + - * / % ^ (or **), comparisons, ?: and !, constants pi, tau, e, and functions sin cos tan asin acos atan atan2 sinh cosh tanh sqrt cbrt abs exp ln log(x[,base]) log10 log2 floor ceil round trunc sign fract pow hypot min max sum mean clamp lerp mod deg rad. Unary minus binds looser than ^ (-2^2 = -4). angleUnit radians (default) or degrees. sweepVariable with sweepValues evaluates over many inputs.",
        &[("expression", "string")],
        &[
            ("variables", "numberMap"),
            ("angleUnit", "string"),
            ("sweepVariable", "string"),
            ("sweepValues", "number[]"),
        ],
    );
    add_extra(
        &mut tools,
        "curve_fit",
        "Least-squares fit of y against x (2 to 100,000 samples, optional weights): linear, polynomial (degree 1-10), logarithmic (a + b·ln x), exponential (a·e^(b·x)) or power (a·x^b). Exponential and power fits minimize error in y with Levenberg-Marquardt from a log-space start. Returns coefficients, equation, R², RMSE, max residual and optional predictions at 'predict' x values.",
        &[("x", "number[]"), ("y", "number[]"), ("model", "string")],
        &[
            ("degree", "integerNumber"),
            ("weights", "number[]"),
            ("predict", "number[]"),
        ],
    );
    add_extra(
        &mut tools,
        "vector_n",
        "Vector operations for 2..32 components: add, subtract, scale, hadamard, divide, min, max, clamp, lerp, project, reject, reflect. Projection/reflect use b as direction/normal.",
        &[("operation", "string"), ("a", "number[]")],
        &[
            ("b", "number[]"),
            ("scalar", "number"),
            ("lower", "number[]"),
            ("upper", "number[]"),
            ("t", "number"),
        ],
    );
    add_extra(
        &mut tools,
        "vector_special",
        "Dimension-specific vector operations: perp and signed_angle (2D), scalar_triple (3D), homogenize (3D to 4D), perspective_divide (4D to 3D).",
        &[("operation", "string"), ("a", "number[]")],
        &[("b", "number[]"), ("c", "number[]"), ("w", "number")],
    );
    add_extra(
        &mut tools,
        "matrix_n",
        "General row-major matrix operations up to 32x32: add, subtract, scale, hadamard, transpose, multiply, matvec, outer. Matrix is {rows,cols,data}.",
        &[("operation", "string")],
        &[
            ("a", "matrix"),
            ("b", "matrix"),
            ("scalar", "number"),
            ("vector", "number[]"),
            ("u", "number[]"),
            ("v", "number[]"),
        ],
    );
    add_extra(
        &mut tools,
        "matrix_properties",
        "Square matrix trace, determinant, rank, frobenius_norm, infinity_norm, condition or inverse. Optional nonnegative SVD tolerance.",
        &[("operation", "string"), ("matrix", "matrix")],
        &[("tolerance", "number")],
    );
    add_extra(
        &mut tools,
        "matrix_factor",
        "LU, QR, Cholesky, SVD, pseudoinverse or least_squares for matrices up to 32x32. Optional rhs and nonnegative SVD tolerance.",
        &[("operation", "string"), ("matrix", "matrix")],
        &[("rhs", "number[]"), ("tolerance", "number")],
    );
    add_extra(
        &mut tools,
        "affine_transform",
        "2D matrices 2x3/3x3 or 3D matrices 3x4/4x4: transform_point, transform_direction, transform_normal, batch_points, expand. Row-major column-vector convention.",
        &[
            ("operation", "string"),
            ("dimension", "integerNumber"),
            ("matrix", "matrix"),
        ],
        &[("vector", "number[]"), ("vectors", "number[][]")],
    );
    add_extra(
        &mut tools,
        "matrix_layout_convert",
        "Convert flat matrix storage between row_major and column_major with optional sourceStride and targetStride (in scalar elements). Padding is zero-filled.",
        &[
            ("rows", "integerNumber"),
            ("cols", "integerNumber"),
            ("data", "number[]"),
            ("sourceLayout", "string"),
            ("targetLayout", "string"),
        ],
        &[
            ("sourceStride", "integerNumber"),
            ("targetStride", "integerNumber"),
        ],
    );
    crate::host_catalog::append(&mut tools);
    tools
}

fn add(tools: &mut Vec<Value>, name: &str, description: &str, params: &[(&str, &str)]) {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for &(key, kind) in params {
        properties.insert(key.to_owned(), schema(kind));
        required.push(key);
    }
    if name == "vector_calculate" {
        properties.insert(
            "b".into(),
            json!({"type":"array","items":{"type":"number"}}),
        );
    }
    if name == "ray_triangle_intersect" {
        properties.insert("cullBackface".into(), json!({"type":"boolean"}));
    }
    tools.push(json!({
        "name": name,
        "description": description,
        "inputSchema": {"type":"object","properties":properties,"required":required,"additionalProperties":false},
        "annotations": {"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}
    }));
}

pub(crate) fn add_extra(
    tools: &mut Vec<Value>,
    name: &str,
    description: &str,
    required: &[(&str, &str)],
    optional: &[(&str, &str)],
) {
    add(tools, name, description, required);
    let properties = tools.last_mut().unwrap()["inputSchema"]["properties"]
        .as_object_mut()
        .unwrap();
    for &(key, kind) in optional {
        properties.insert(key.to_owned(), schema(kind));
    }
}

fn schema(kind: &str) -> Value {
    match kind {
        "number" => json!({"type":"number"}),
        "integerNumber" => json!({"type":"integer"}),
        "boolean" => json!({"type":"boolean"}),
        "number[]" => json!({"type":"array","items":{"type":"number"}}),
        "string[]" => json!({"type":"array","items":{"type":"string"}}),
        "stringMap" => json!({"type":"object","additionalProperties":{"type":"string"}}),
        "jsonPatch" => {
            json!({"type":"array","minItems":1,"maxItems":1000,"items":{"type":"object","properties":{
            "op":{"type":"string","enum":["add","remove","replace","move","copy","test"]},
            "path":{"type":"string"},"from":{"type":"string"},"value":{}},"required":["op","path"]}})
        }
        "fileOpenMode" => json!({"type":"string","enum":["open","reveal"],"default":"open"}),
        "pipelineSteps" => {
            json!({"type":"array","minItems":1,"maxItems":32,"items":{"type":"object","properties":{
            "executable":{"type":"string"},"arguments":{"type":"array","items":{"type":"string"}},
            "cwd":{"type":"string"},"env":{"type":"object","additionalProperties":{"type":"string"}},
            "timeoutMs":{"type":"integer","minimum":1,"maximum":300000},
            "maxOutputBytes":{"type":"integer","minimum":0,"maximum":4194304}},
            "required":["executable"],"additionalProperties":false}})
        }
        "number[][]" => json!({"type":"array","items":{"type":"array","items":{"type":"number"}}}),
        "number[][][]" => {
            json!({"type":"array","items":{"type":"array","items":{"type":"array","items":{"type":"number"}}}})
        }
        "numberMap" => json!({"type":"object","additionalProperties":{"type":"number"}}),
        "object[]" => json!({"type":"array","maxItems":128,"items":{"type":"object",
            "properties":{"name":{"type":"string"},"virtualAddress":{"type":"string"},
                "virtualSize":{"type":"string"},"pointerToRawData":{"type":"string"},
                "sizeOfRawData":{"type":"string"}},
            "required":["virtualAddress","virtualSize","pointerToRawData","sizeOfRawData"],
            "additionalProperties":false}}),
        "scalar[]" => {
            json!({"type":"array","items":{"oneOf":[{"type":"number"},{"type":"string"}]}})
        }
        "fraction" => {
            json!({"type":"object","properties":{"numerator":{"type":"string"},"denominator":{"type":"string"}},"required":["numerator","denominator"]})
        }
        "matrix" => {
            json!({"type":"object","properties":{"rows":{"type":"integer","minimum":1,"maximum":32},
            "cols":{"type":"integer","minimum":1,"maximum":32},"data":{"type":"array","items":{"type":"number"},"maxItems":1024}},
            "required":["rows","cols","data"],"additionalProperties":false})
        }
        "transformSteps" => {
            json!({"type":"array","minItems":1,"maxItems":32,"items":{"type":"object","properties":{
            "op":{"type":"string","enum":["xor","add","sub","rol","ror","not","reverse","slice",
                "hex_encode","hex_decode","base64_encode","base64_decode","base32_encode","base32_decode",
                "zlib_compress","zlib_decompress","deflate_compress","deflate_decompress",
                "gzip_compress","gzip_decompress","lz4_block_compress","lz4_block_decompress",
                "lz4_frame_compress","lz4_frame_decompress"]},
            "key":{"type":"string","description":"xor: hex key bytes, repeated over the data."},
            "value":{"type":"integer","minimum":0,"maximum":255,"description":"add/sub: byte amount (wrapping)."},
            "bits":{"type":"integer","minimum":0,"maximum":7,"description":"rol/ror: per-byte rotation."},
            "offset":{"type":"integer","minimum":0,"description":"slice: start byte."},
            "length":{"type":"integer","minimum":0,"description":"slice: byte count (default: to the end)."},
            "size":{"type":"integer","minimum":0,"description":"lz4_block_decompress: exact decompressed size."}},
            "required":["op"],"additionalProperties":false}})
        }
        "integer" => {
            json!({"type":"string","description":"Decimal, 0x hexadecimal or 0b binary integer; optional leading minus sign."})
        }
        _ => json!({"type":"string"}),
    }
}
