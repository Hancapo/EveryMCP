use super::{io_error, optional_bool, optional_str, optional_u64, required_str, required_u64};
use hickory_resolver::{Resolver, proto::rr::RecordType};
use native_tls::TlsConnector;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
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

pub fn download_file(args: &Value) -> Result<Value, String> {
    let url = required_str(args, "url")?;
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("'url' must use http or https.".into());
    }
    let path = Path::new(required_str(args, "path")?);
    let overwrite = optional_bool(args, "overwrite", false)?;
    if path.exists() && !overwrite {
        return Err("Destination already exists.".into());
    }
    let max_bytes = optional_u64(args, "maxBytes", 512 * 1024 * 1024, 4 * 1024 * 1024 * 1024)?;
    if max_bytes == 0 {
        return Err("'maxBytes' must be positive.".into());
    }
    let expected = optional_str(args, "expectedSha256")?;
    if let Some(value) = expected
        && (value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("'expectedSha256' must contain 64 hex digits.".into());
    }
    let timeout = optional_u64(args, "timeoutMs", 300000, 3600000)?;
    let agent = Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(timeout)))
        .http_status_as_error(false)
        .max_redirects(5)
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| io_error("Download failed", e))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("Download returned HTTP {status}."));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| io_error("Cannot create download temporary file", e))?;
    let mut reader = response.body_mut().as_reader();
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = [0u8; 65536];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| io_error("Cannot read download", e))?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or("Download size overflow.")?;
        if total > max_bytes {
            return Err("Download exceeds maxBytes.".into());
        }
        hasher.update(&buf[..n]);
        temp.write_all(&buf[..n])
            .map_err(|e| io_error("Cannot write download", e))?;
    }
    let digest = super::files::digest_hex(&hasher.finalize());
    if expected.is_some_and(|value| !digest.eq_ignore_ascii_case(value)) {
        return Err("Downloaded SHA-256 does not match expectedSha256.".into());
    }
    temp.flush()
        .map_err(|e| io_error("Cannot flush download", e))?;
    let result = if overwrite {
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    };
    result.map_err(|e| io_error("Cannot persist download", e.error))?;
    Ok(
        json!({"url":url,"path":path.to_string_lossy(),"status":status,"bytesWritten":total,"sha256":digest}),
    )
}

pub fn connect_tcp(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<(TcpStream, SocketAddr), String> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|e| io_error("Cannot resolve host", e))?;
    let mut last_error = "No addresses found.".to_owned();
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok((stream, address)),
            Err(error) => last_error = error.to_string(),
        }
    }
    Err(last_error)
}

pub fn probe(args: &Value) -> Result<Value, String> {
    let host = required_str(args, "host")?;
    let port = required_u64(args, "port")?;
    if port == 0 || port > 65535 {
        return Err("'port' must be between 1 and 65535.".into());
    }
    let protocol = optional_str(args, "protocol")?.unwrap_or("tcp");
    if protocol != "tcp" && protocol != "tls" {
        return Err("'protocol' must be tcp or tls.".into());
    }
    let timeout = optional_u64(args, "timeoutMs", 5000, 30000)?;
    let start = std::time::Instant::now();
    let (stream, address) = match connect_tcp(host, port as u16, Duration::from_millis(timeout)) {
        Ok(connection) => connection,
        Err(error) => {
            return Ok(
                json!({"host":host,"port":port,"protocol":protocol,"connected":false,"error":error,"elapsedMs":start.elapsed().as_millis()}),
            );
        }
    };
    if protocol == "tcp" {
        return Ok(
            json!({"host":host,"port":port,"protocol":protocol,"connected":true,"address":address.to_string(),"elapsedMs":start.elapsed().as_millis()}),
        );
    }
    stream
        .set_read_timeout(Some(Duration::from_millis(timeout)))
        .map_err(|e| io_error("Cannot set read timeout", e))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(timeout)))
        .map_err(|e| io_error("Cannot set write timeout", e))?;
    let connector = TlsConnector::new().map_err(|e| io_error("Cannot create TLS connector", e))?;
    match connector.connect(host, stream) {
        Ok(tls) => {
            let certificate = tls.peer_certificate().ok().flatten();
            let sha256 = certificate.and_then(|cert| cert.to_der().ok()).map(|der| {
                use sha2::Digest;
                super::files::digest_hex(&sha2::Sha256::digest(der))
            });
            Ok(
                json!({"host":host,"port":port,"protocol":protocol,"connected":true,"address":address.to_string(),"peerCertificateSha256":sha256,"elapsedMs":start.elapsed().as_millis()}),
            )
        }
        Err(error) => Ok(
            json!({"host":host,"port":port,"protocol":protocol,"connected":false,"address":address.to_string(),"error":error.to_string(),"elapsedMs":start.elapsed().as_millis()}),
        ),
    }
}

pub fn dns_query(args: &Value) -> Result<Value, String> {
    let name = required_str(args, "name")?;
    let kind = optional_str(args, "recordType")?
        .unwrap_or("A")
        .to_ascii_uppercase();
    let record_type = match kind.as_str() {
        "A" => RecordType::A,
        "AAAA" => RecordType::AAAA,
        "CNAME" => RecordType::CNAME,
        "MX" => RecordType::MX,
        "NS" => RecordType::NS,
        "TXT" => RecordType::TXT,
        "SRV" => RecordType::SRV,
        _ => return Err("Unsupported DNS record type.".into()),
    };
    let timeout = optional_u64(args, "timeoutMs", 5000, 30000)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| io_error("Cannot start DNS runtime", e))?;
    runtime.block_on(async {
        let resolver = Resolver::builder_tokio()
            .map_err(|e| io_error("Cannot load DNS configuration", e))?
            .build()
            .map_err(|e| io_error("Cannot create DNS resolver", e))?;
        let lookup = tokio::time::timeout(
            Duration::from_millis(timeout),
            resolver.lookup(name, record_type),
        )
        .await
        .map_err(|_| "DNS query timed out.".to_owned())?
        .map_err(|e| io_error("DNS query failed", e))?;
        let records: Vec<Value> = lookup
            .answers()
            .iter()
            .map(|record| {
                json!({
                    "name":record.name.to_string(), "type":kind, "ttl":record.ttl,
                    "value":record.data.to_string()
                })
            })
            .collect();
        Ok(json!({"name":name,"recordType":kind,"records":records}))
    })
}
