use super::{io_error, optional_str, optional_u64, required_str};
use serde_json::{Value, json};
use std::io::Read;
use std::time::Duration;
use ureq::{Agent, http};

pub fn request(args: &Value) -> Result<Value, String> {
    let url = required_str(args, "url")?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("'url' must use http or https.".into());
    }
    let method = optional_str(args, "method")?
        .unwrap_or("GET")
        .to_ascii_uppercase();
    if !["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
        return Err("Unsupported HTTP method.".into());
    }
    let timeout = optional_u64(args, "timeoutMs", 30000, 300000)?;
    let max_body = optional_u64(args, "maxBodyBytes", 1024 * 1024, 4 * 1024 * 1024)? as usize;
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(timeout)))
        .http_status_as_error(false)
        .max_redirects(5)
        .build();
    let agent = config.new_agent();
    let mut builder = http::Request::builder().method(method.as_str()).uri(url);
    if let Some(headers) = args.get("headers") {
        let headers = headers
            .as_object()
            .ok_or("'headers' must be an object of strings.")?;
        for (key, value) in headers {
            builder = builder.header(
                key.as_str(),
                value.as_str().ok_or("Header values must be strings.")?,
            );
        }
    }
    let body = optional_str(args, "body")?
        .unwrap_or("")
        .as_bytes()
        .to_vec();
    let request = builder
        .body(body)
        .map_err(|e| io_error("Invalid HTTP request", e))?;
    let mut response = agent
        .run(request)
        .map_err(|e| io_error("HTTP request failed", e))?;
    let status = response.status().as_u16();
    let headers: Vec<Value> = response
        .headers()
        .iter()
        .map(|(name, value)| {
            json!({
                "name":name.as_str(), "value":value.to_str().unwrap_or("")
            })
        })
        .collect();
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(max_body as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error("Cannot read HTTP response", e))?;
    let truncated = bytes.len() > max_body;
    bytes.truncate(max_body);
    let (body, encoding) = match String::from_utf8(bytes.clone()) {
        Ok(text) => (text, "utf8"),
        Err(_) => (crate::bytes::format_hex(&bytes), "hex"),
    };
    Ok(
        json!({"status":status,"headers":headers,"body":body,"bodyEncoding":encoding,"bodyBytes":bytes.len(),"truncated":truncated}),
    )
}
