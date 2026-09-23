use serde_json::{Map, Value, json};

mod geometry;
mod geometry_extra;
mod linear_extra;
mod numeric_extra;
mod re;
mod re_extra;
mod scalar;
mod vector_extra;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    if !args.is_object() {
        return Err("Tool arguments must be an object.".into());
    }
    match name {
        "process_start" | "process_run" | "process_get" | "process_list" | "process_tree"
        | "process_wait" | "process_output" | "process_stop" | "process_modules" | "file_find"
        | "text_search" | "file_stat" | "file_hash" | "file_read_range" | "file_write_atomic"
        | "file_copy_move" | "port_owner" | "service_get" | "service_control"
        | "eventlog_query" | "registry_read" | "environment_get" | "system_info"
        | "archive_create" | "archive_extract" | "powershell_run" | "process_input"
        | "wait_for" | "file_patch" | "http_request" | "network_probe" | "dns_query"
        | "executable_resolve" | "directory_manifest" | "file_signature" => {
            crate::host::execute(name, args)
        }
        "add" | "subtract" | "multiply" | "division" | "modulo" | "sum" | "mean" | "median"
        | "mode" | "min" | "max" | "floor" | "ceiling" | "round" | "sin" | "arcsin" | "cos"
        | "arccos" | "tan" | "arctan" | "radiansToDegrees" | "degreesToRadians" => {
            scalar::execute(name, args)
        }
        "integer_convert" | "bitfield_extract" | "bitfield_insert" | "address_translate"
        | "align_address" | "relative_target" | "ieee754_decode" => re::execute(name, args),
        "vector_calculate"
        | "matrix_transform"
        | "matrix_multiply"
        | "matrix_inverse"
        | "quaternion_rotate"
        | "barycentric_2d"
        | "ray_triangle_intersect" => geometry::execute(name, args),
        "pe_address_map"
        | "binary_unpack"
        | "binary_pack"
        | "bitwise_word"
        | "fixed_width_alu"
        | "leb128_codec"
        | "x86_effective_address"
        | "pe_relocation_apply"
        | "packed_vertex_decode"
        | "crc_compute" => re_extra::execute(name, args),
        "compose_transform"
        | "decompose_transform"
        | "rotation_convert"
        | "quaternion_slerp"
        | "project_unproject"
        | "ray_primitive_intersect"
        | "closest_point"
        | "segment_intersect_2d"
        | "polygon_measure_2d"
        | "frustum_test" => geometry_extra::execute(name, args),
        "power_root_log"
        | "exact_fraction"
        | "gcd_extended"
        | "modular_arithmetic"
        | "combinatorics_exact"
        | "complex_calculate"
        | "polynomial_evaluate"
        | "linear_system_solve"
        | "descriptive_statistics"
        | "interpolate_samples" => numeric_extra::execute(name, args),
        "vector_n" | "vector_special" => vector_extra::execute(name, args),
        "matrix_n"
        | "matrix_properties"
        | "matrix_factor"
        | "affine_transform"
        | "matrix_layout_convert" => linear_extra::execute(name, args),
        _ => Err(format!("Unknown tool '{name}'.")),
    }
}

pub(super) fn value(number: f64) -> Result<Value, String> {
    if !number.is_finite() {
        return Err("Result is not finite.".into());
    }
    Ok(json!({"value":number}))
}

pub(super) fn array(name: &str, numbers: Vec<f64>) -> Result<Value, String> {
    let mut map = Map::new();
    map.insert(name.into(), checked_array(numbers)?);
    Ok(Value::Object(map))
}

pub(super) fn checked_array(numbers: Vec<f64>) -> Result<Value, String> {
    if numbers.iter().any(|n| !n.is_finite()) {
        return Err("Result is not finite.".into());
    }
    Ok(json!(numbers))
}
