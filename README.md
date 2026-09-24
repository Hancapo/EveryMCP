# EveryMCP

A native Rust MCP server with **139 tools** for mathematics, reverse engineering, geometry, and local system automation. It includes the 22 operations from [math-mcp](https://github.com/EthanHenrickson/math-mcp) with their original names and arguments; no code or dependencies are copied from that project.

## Build

```sh
cargo build --release --locked
```

Use `target/release/EveryMCP.exe` on Windows or `target/release/EveryMCP` on Linux and macOS. The server uses MCP over stdio and supports both legacy `2025-06-18` initialization and `2026-07-28` per-request metadata.

Example client configuration:

```json
{
  "mcpServers": {
    "everymcp": {
      "command": "C:\\path\\to\\EveryMCP.exe"
    }
  }
}
```

## Tools

| Area | Included operations |
| --- | --- |
| Mathematics | Arithmetic, statistics, trigonometry, exact fractions, modular arithmetic, complex numbers, interpolation, and numerical analysis. |
| Reverse engineering | Integer and bit operations, address translation, PE mapping and relocations, dumping the loaded main executable, binary packing, floating-point decoding, and checksums. |
| Geometry and linear algebra | 2D/3D intersections, transforms, quaternions, vector operations, and matrices up to 32 × 32. |
| Processes | Start, run, inspect, list, send stdin, read output incrementally, wait for, and stop processes; inspect loaded modules. |
| Files and ZIP | Find, compare, tail, hash, transcode, inspect images, search binary patterns, write, patch, copy, trash, create ZIP archives, inspect them, and extract all or selected entries. |
| HTTP and readiness | Make bounded HTTP requests, stream verified downloads to disk, query DNS, probe TCP/TLS, and wait for files, process exits, TCP ports, or HTTP status. |
| Inspection | Resolve executables, inventory directories with hashes, inspect PE images, and inspect Windows file signatures. |
| Windows | Open files or reveal them in Explorer; inspect hardware, file lock holders, ports, services, tasks, event logs, ACLs, registry values, and environment variables. |
| Telemetry | Sample CPU, memory, network transfer, and disk capacity. |
| Workspace | Discover projects and Git status, run command pipelines, capture environments, and wait for file changes. |
| Structured data | Query and compare JSON, TOML, YAML, XML and CSV; inventory declared and resolved dependencies. |
| Artifacts and diagnostics | Save and compare hashed directory snapshots; parse compiler diagnostics and JUnit, TRX or TAP test reports. |

See [the tool guide](docs/TOOLS.md) for the purpose and a practical use case for every tool.

Call `tools/list` for the complete tool catalog and JSON schemas. For example:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"process_run","arguments":{"executable":"git","arguments":["status","--short"],"cwd":"C:\\project"}}}
```

## Platform and behavior

The math, file, and core process tools are cross-platform. Windows administration tools require Windows PowerShell. Host operations run with the permissions of the account that launches EveryMCP; `powershell_run` executes the supplied script. Child process output is captured so it cannot corrupt the MCP stream. Each input message is limited to 1 MiB.

## Verify

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo build --locked
python tests/test_mcp.py
cargo build --release --locked
python tests/test_mcp.py target/release/EveryMCP.exe
```

On Linux and macOS, omit `.exe` from the final command.
