mod bytes;
mod catalog;
mod exact;
mod geometry_math;
mod host;
mod host_catalog;
mod input;
mod integer;
mod linear;
mod math;
mod ops;

use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

const LEGACY_PROTOCOL_VERSION: &str = "2025-06-18";
const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
const MAX_MESSAGE_LENGTH: usize = 1024 * 1024;

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    let tools = catalog::list();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let response = if line.len() > MAX_MESSAGE_LENGTH {
            Some(error(Value::Null, -32700, "Message exceeds 1 MiB."))
        } else {
            process(&line, &tools)
        };
        if let Some(response) = response {
            writeln!(writer, "{response}")?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn process(line: &str, tools: &[Value]) -> Option<Value> {
    let request: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return Some(error(Value::Null, -32700, "Invalid JSON.")),
    };
    if !request.is_object()
        || request.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || request.get("method").and_then(Value::as_str).is_none()
    {
        return Some(error(Value::Null, -32600, "Invalid JSON-RPC request."));
    }
    let id = request.get("id")?.clone(); // Notifications have no response.
    let method = request["method"].as_str().unwrap();
    let modern = match modern_request(&request, method, &id) {
        Ok(value) => value,
        Err(response) => return Some(response),
    };
    Some(match method {
        "server/discover" => result(
            id,
            json!({
                "supportedVersions":[MODERN_PROTOCOL_VERSION,LEGACY_PROTOCOL_VERSION],
                "capabilities":{"tools":{"listChanged":false}},
                "ttlMs":300000,
                "cacheScope":"public"
            }),
            true,
        ),
        "initialize" if modern => error(
            id,
            -32601,
            "Initialize is not available in this protocol version.",
        ),
        "initialize" => result(
            id,
            json!({
                "protocolVersion": LEGACY_PROTOCOL_VERSION,
                "capabilities": {"tools":{"listChanged":false}},
            "serverInfo": {"name":"EveryMCP","version":env!("CARGO_PKG_VERSION")}
            }),
            false,
        ),
        "ping" => result(id, json!({}), modern),
        "tools/list"
            if request
                .get("params")
                .and_then(|value| value.get("cursor"))
                .is_some() =>
        {
            error(
                id,
                -32602,
                "This server returns the full tool list; cursors are not accepted.",
            )
        }
        "tools/list" => result(id, json!({"tools":tools}), modern),
        "tools/call" => call(id, &request, tools, modern),
        _ => error(id, -32601, &format!("Method '{method}' not found.")),
    })
}

fn modern_request(request: &Value, method: &str, id: &Value) -> Result<bool, Value> {
    let meta = request.get("params").and_then(|params| params.get("_meta"));
    if meta.is_none() && method != "server/discover" {
        return Ok(false);
    }
    let meta = meta
        .and_then(Value::as_object)
        .ok_or_else(|| error(id.clone(), -32602, "Missing or invalid request _meta."))?;
    let version = meta
        .get("io.modelcontextprotocol/protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            error(
                id.clone(),
                -32602,
                "Missing protocol version in request _meta.",
            )
        })?;
    if version != MODERN_PROTOCOL_VERSION {
        return Err(error_data(
            id.clone(),
            -32022,
            "Unsupported protocol version.",
            json!({"requested":version,"supported":[MODERN_PROTOCOL_VERSION,LEGACY_PROTOCOL_VERSION]}),
        ));
    }
    if !meta
        .get("io.modelcontextprotocol/clientCapabilities")
        .is_some_and(Value::is_object)
    {
        return Err(error(
            id.clone(),
            -32602,
            "Missing client capabilities in request _meta.",
        ));
    }
    Ok(true)
}

fn call(id: Value, request: &Value, tools: &[Value], modern: bool) -> Value {
    let operation = (|| -> Result<Value, String> {
        let params = input::property(request, "params")?;
        let name = input::string(params, "name")?;
        if !tools.iter().any(|tool| tool["name"] == name) {
            return Err(format!("Unknown tool '{name}'."));
        }
        let args = params.get("arguments").unwrap_or(&Value::Null);
        ops::execute(name, args)
    })();
    let (text, is_error) = match operation {
        Ok(output) => (output.to_string(), false),
        Err(message) => (message, true),
    };
    result(
        id,
        json!({"content":[{"type":"text","text":text}],"isError":is_error}),
        modern,
    )
}

fn result(id: Value, mut output: Value, modern: bool) -> Value {
    if modern && let Some(map) = output.as_object_mut() {
        map.insert("resultType".into(), json!("complete"));
        map.insert(
            "_meta".into(),
            json!({"io.modelcontextprotocol/serverInfo":{
                "name":"EveryMCP","version":env!("CARGO_PKG_VERSION")
            }}),
        );
    }
    json!({"jsonrpc":"2.0","id":id,"result":output})
}
fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn error_data(id: Value, code: i32, message: &str, data: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message,"data":data}})
}
