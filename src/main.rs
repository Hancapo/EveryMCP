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

const PROTOCOL_VERSION: &str = "2025-06-18";
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
    Some(match method {
        "initialize" => result(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools":{"listChanged":false}},
            "serverInfo": {"name":"EveryMCP","version":env!("CARGO_PKG_VERSION")}
            }),
        ),
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({"tools":tools})),
        "tools/call" => call(id, &request, tools),
        _ => error(id, -32601, &format!("Method '{method}' not found.")),
    })
}

fn call(id: Value, request: &Value, tools: &[Value]) -> Value {
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
    )
}

fn result(id: Value, output: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":output})
}
fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
