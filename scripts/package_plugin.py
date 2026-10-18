"""Package the Windows x64 plugin using the built server's archive tool."""

import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import tomllib
import zipfile


def package():
    repo = Path(__file__).resolve().parents[1]
    manifest = json.loads((repo / "plugin.json").read_text(encoding="utf-8"))
    cargo = tomllib.loads((repo / "Cargo.toml").read_text(encoding="utf-8"))
    if manifest["version"] != cargo["package"]["version"]:
        raise ValueError("Plugin and Cargo versions must match.")
    interface = manifest["extensions"]["com.openai"]["interface"]
    if len(interface["shortDescription"]) > 30:
        raise ValueError("Plugin subtitle must not exceed 30 characters.")
    binary = repo / "target" / "release" / "EveryMCP.exe"
    with binary.open("rb") as source:
        header = source.read(64)
        if len(header) != 64 or header[:2] != b"MZ":
            raise ValueError("Build the Windows x64 release executable first.")
        source.seek(struct.unpack_from("<I", header, 60)[0])
        if source.read(6) != b"PE\0\0\x64\x86":
            raise ValueError("Plugin executable must be a Windows x64 PE.")
    artifacts = repo / "artifacts"
    artifacts.mkdir(exist_ok=True)
    archive = artifacts / f"everymcp-{manifest['version']}-windows-x64.zip"
    with tempfile.TemporaryDirectory(prefix="plugin staging ", dir=artifacts) as staging:
        staging = Path(staging).resolve()
        if not staging.is_relative_to(artifacts.resolve()):
            raise ValueError("Staging directory escaped the artifacts directory.")
        root = staging / manifest["name"]
        for relative in ("plugin.json", "mcp.json", "assets/everymcp.svg", "README.md", "docs/TOOLS.md"):
            destination = root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(repo / relative, destination)
        (root / "bin").mkdir()
        shutil.copy2(binary, root / "bin" / "EveryMCP.exe")
        config = json.loads((root / "mcp.json").read_text(encoding="utf-8"))["mcpServers"]["everymcp"]
        command = (root / config["command"]).resolve()
        if not command.is_relative_to(root):
            raise ValueError("Plugin command must stay inside the package.")
        cwd = config["cwd"].replace("${PLUGIN_ROOT}", str(root))
        requests = [
            {"method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "plugin-packager", "version": "1.0.0"}}},
            {"method": "tools/list"},
            {"method": "tools/call", "params": {"name": "add", "arguments": {"firstNumber": 2, "secondNumber": 3}}},
            {"method": "tools/call", "params": {"name": "archive_create", "arguments": {"archive": str(archive), "paths": [str(root)], "overwrite": True}}},
        ]
        messages = "".join(json.dumps({"jsonrpc": "2.0", "id": index, **request}) + "\n" for index, request in enumerate(requests))
        completed = subprocess.run([str(command), *config.get("args", [])], cwd=cwd, input=messages, text=True, capture_output=True, check=True, timeout=60)
        responses = [json.loads(line) for line in completed.stdout.splitlines()]
        if len(responses) != len(requests) or any("error" in item or item.get("result", {}).get("isError") for item in responses):
            raise ValueError(f"Plugin server verification failed: {completed.stdout} {completed.stderr}")
        if responses[0]["result"]["serverInfo"]["version"] != manifest["version"]:
            raise ValueError("Release executable is stale; rebuild it before packaging.")
        tools = responses[1]["result"]["tools"]
        if len(tools) != 154 or len({tool["name"] for tool in tools}) != len(tools):
            raise ValueError("Plugin tool catalog does not match this release.")
        if json.loads(responses[2]["result"]["content"][0]["text"])["value"] != 5:
            raise ValueError("Packaged server failed its arithmetic check.")
        with zipfile.ZipFile(archive) as packed:
            expected = {f"everymcp/{path.relative_to(root).as_posix()}" for path in root.rglob("*") if path.is_file()}
            actual = {item.filename for item in packed.infolist() if not item.is_dir()}
            if actual != expected or packed.testzip() is not None:
                raise ValueError("Archive contents or CRC verification failed.")
        print(f"Verified {len(tools)} tools; wrote {archive}")


if __name__ == "__main__":
    package()
