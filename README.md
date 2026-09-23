# EveryMCP

A native Rust MCP server with **103 tools** for mathematics, reverse engineering, geometry, and local system automation. It includes the 22 operations from [math-mcp](https://github.com/EthanHenrickson/math-mcp) with their original names and arguments; no code or dependencies are copied from that project.

## Build

```sh
cargo build --release --locked
```

Use `target/release/EveryMCP.exe` on Windows or `target/release/EveryMCP` on Linux and macOS. The server uses MCP over stdio.

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
| Reverse engineering | Integer and bit operations, address translation, PE mapping and relocations, binary packing, floating-point decoding, and checksums. |
| Geometry and linear algebra | 2D/3D intersections, transforms, quaternions, vector operations, and matrices up to 32 × 32. |
| Processes | Start, run, inspect, list, send stdin, read output incrementally, wait for, and stop processes; inspect loaded modules. |
| Files and ZIP | Find and search files, read byte ranges, hash, write, patch with preconditions, copy, move, create ZIP archives, and extract them. |
| HTTP and readiness | Make bounded HTTP requests and wait for files, process exits, TCP ports, or HTTP status. |
| Windows | Port ownership, services, event logs, registry values, environment variables, and PowerShell execution. |

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
