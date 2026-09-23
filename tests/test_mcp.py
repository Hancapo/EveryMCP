"""End-to-end MCP contract tests. Run with: python tests/test_mcp.py [path-to-exe]."""
import json
import os
import hashlib
import http.server
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
import zipfile
from pathlib import Path


DEFAULT_EXE = os.path.join("target", "debug", "EveryMCP.exe" if os.name == "nt" else "EveryMCP")
EXE = sys.argv.pop(1) if len(sys.argv) > 1 else DEFAULT_EXE


class McpTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.process = subprocess.Popen([EXE], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True, bufsize=1)
        cls.next_id = 0

    @classmethod
    def tearDownClass(cls):
        cls.process.stdin.close()
        cls.process.wait(timeout=5)
        cls.process.stdout.close()
        cls.process.stderr.close()

    def request(self, method, params=None):
        self.__class__.next_id += 1
        request_id = self.next_id
        message = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            message["params"] = params
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        self.assertTrue(line, "server exited without a response")
        response = json.loads(line)
        self.assertEqual(response["id"], request_id)
        return response

    def call(self, name, arguments):
        response = self.request("tools/call", {"name": name, "arguments": arguments})
        self.assertNotIn("error", response)
        result = response["result"]
        self.assertFalse(result.get("isError", False), result)
        return json.loads(result["content"][0]["text"])

    @staticmethod
    def modern_params(**extra):
        return {"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
                          "io.modelcontextprotocol/clientCapabilities": {}}, **extra}

    def test_modern_protocol_discovery_and_tools(self):
        discovered = self.request("server/discover", self.modern_params())["result"]
        self.assertEqual(discovered["resultType"], "complete")
        self.assertIn("2026-07-28", discovered["supportedVersions"])
        self.assertIn("2025-06-18", discovered["supportedVersions"])
        self.assertEqual(discovered["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "EveryMCP")
        listed = self.request("tools/list", self.modern_params())["result"]
        self.assertEqual(listed["resultType"], "complete")
        self.assertEqual(len(listed["tools"]), 124)
        called = self.request("tools/call", self.modern_params(name="add", arguments={
            "firstNumber": 2, "secondNumber": 3}))["result"]
        self.assertEqual(called["resultType"], "complete")
        self.assertEqual(json.loads(called["content"][0]["text"])["value"], 5)
        pinged = self.request("ping", self.modern_params())["result"]
        self.assertEqual(pinged["resultType"], "complete")
        failed = self.request("tools/call", self.modern_params(name="division", arguments={
            "numerator": 1, "denominator": 0}))["result"]
        self.assertEqual(failed["resultType"], "complete")
        self.assertTrue(failed["isError"])

    def test_modern_protocol_validation(self):
        missing = self.request("tools/list", {"_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28"}})
        self.assertEqual(missing["error"]["code"], -32602)
        unsupported = self.request("tools/list", {"_meta": {
            "io.modelcontextprotocol/protocolVersion": "2099-01-01",
            "io.modelcontextprotocol/clientCapabilities": {}}})
        self.assertEqual(unsupported["error"]["code"], -32022)
        self.assertEqual(unsupported["error"]["data"]["requested"], "2099-01-01")
        self.assertEqual(unsupported["error"]["data"]["supported"], ["2026-07-28", "2025-06-18"])
        bad_cursor = self.request("tools/list", self.modern_params(cursor="invalid"))
        self.assertEqual(bad_cursor["error"]["code"], -32602)

    def test_initialize_and_catalog(self):
        response = self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                               "clientInfo": {"name": "test", "version": "1"}})
        self.assertEqual(response["result"]["protocolVersion"], "2025-06-18")
        self.assertEqual(response["result"]["serverInfo"]["name"], "EveryMCP")
        tools = self.request("tools/list")["result"]["tools"]
        names = {tool["name"] for tool in tools}
        self.assertEqual(names, {"add", "subtract", "multiply", "division", "sum", "modulo", "floor",
                                 "ceiling", "round", "mean", "median", "mode", "min", "max", "sin",
                                 "arcsin", "cos", "arccos", "tan", "arctan", "radiansToDegrees",
                                 "degreesToRadians", "integer_convert", "bitfield_extract", "bitfield_insert",
                                 "address_translate", "align_address", "relative_target", "ieee754_decode",
                                 "vector_calculate", "matrix_transform", "matrix_multiply", "matrix_inverse",
                                 "quaternion_rotate", "barycentric_2d", "ray_triangle_intersect",
                                 "pe_address_map", "binary_unpack", "binary_pack", "bitwise_word",
                                 "fixed_width_alu", "leb128_codec", "x86_effective_address",
                                 "pe_relocation_apply", "packed_vertex_decode", "crc_compute",
                                 "compose_transform", "decompose_transform", "rotation_convert",
                                 "quaternion_slerp", "project_unproject", "ray_primitive_intersect",
                                 "closest_point", "segment_intersect_2d", "polygon_measure_2d",
                                 "frustum_test", "power_root_log", "exact_fraction", "gcd_extended",
                                 "modular_arithmetic", "combinatorics_exact", "complex_calculate",
                                 "polynomial_evaluate", "linear_system_solve", "descriptive_statistics",
                                 "interpolate_samples", "vector_n", "vector_special", "matrix_n",
                                 "matrix_properties", "matrix_factor", "affine_transform",
                                 "matrix_layout_convert", "process_start", "process_run", "process_get",
                                 "process_list", "process_tree", "process_wait", "process_output",
                                 "process_stop", "process_modules", "file_find", "text_search",
                                 "file_stat", "file_hash", "file_read_range", "file_write_atomic",
                                 "file_copy_move", "port_owner", "service_get", "service_control",
                                 "eventlog_query", "registry_read", "environment_get", "system_info",
                                 "archive_create", "archive_extract", "powershell_run",
                                 "process_input", "wait_for", "file_patch", "http_request",
                                 "network_probe", "dns_query", "executable_resolve",
                                 "directory_manifest", "file_signature", "scheduled_task",
                            "eventlog_follow", "performance_sample", "acl_get",
                            "workspace_scan", "repo_status_batch", "command_pipeline",
                            "environment_snapshot", "file_watch", "structured_data_query",
                            "structured_data_diff", "dependency_inventory", "artifact_manifest",
                            "manifest_diff", "diagnostics_parse", "test_results_parse"})
        for tool in tools:
            self.assertEqual(tool["inputSchema"]["type"], "object")
        pe = next(tool for tool in tools if tool["name"] == "pe_address_map")
        section_required = set(pe["inputSchema"]["properties"]["sections"]["items"]["required"])
        self.assertEqual(section_required, {"virtualAddress", "virtualSize", "pointerToRawData", "sizeOfRawData"})

    def test_host_files_and_archives(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "a.txt"
            written = self.call("file_write_atomic", {"path": str(source), "content": "alpha\nbeta\n"})
            self.assertEqual(written["bytesWritten"], 11)
            self.assertEqual(self.call("file_stat", {"path": str(source)})["sizeBytes"], 11)
            self.assertEqual(self.call("file_read_range", {"path": str(source), "offset": 6,
                                                           "length": 4, "encoding": "utf8"})["data"], "beta")
            self.assertEqual(len(self.call("file_hash", {"path": str(source)})["digest"]), 64)
            self.assertEqual(self.call("file_hash", {"path": str(source)})["digest"],
                             hashlib.sha256(b"alpha\nbeta\n").hexdigest())
            self.assertEqual(self.call("file_read_range", {"path": str(source), "offset": 0,
                                                           "length": 2, "encoding": "hex"})["data"], "616c")
            self.assertEqual(len(self.call("file_find", {"root": str(root), "pattern": "*.txt"})["entries"]), 1)
            matches = self.call("text_search", {"root": str(root), "pattern": "beta"})["matches"]
            self.assertEqual(matches[0]["line"], 2)
            copied = root / "b.txt"
            self.call("file_copy_move", {"source": str(source), "destination": str(copied),
                                         "operation": "copy"})
            empty = root / "empty"
            empty.mkdir()
            archive = root / "test.zip"
            self.call("archive_create", {"archive": str(archive), "paths": [str(source), str(copied),
                                                                             str(empty)]})
            extracted = root / "out"
            self.call("archive_extract", {"archive": str(archive), "destination": str(extracted)})
            self.assertEqual((extracted / "a.txt").read_text(), "alpha\nbeta\n")
            self.assertTrue((extracted / "empty").is_dir())
            moved = root / "moved.txt"
            self.call("file_copy_move", {"source": str(copied), "destination": str(moved),
                                         "operation": "move"})
            self.assertFalse(copied.exists())
            self.assertTrue(moved.exists())
            moved.write_text("old")
            self.call("file_copy_move", {"source": str(source), "destination": str(moved),
                                         "operation": "copy", "overwrite": True})
            self.assertEqual(moved.read_text(), "alpha\nbeta\n")
            denied = self.request("tools/call", {"name": "file_write_atomic",
                                                  "arguments": {"path": str(source), "content": "new"}})
            self.assertTrue(denied["result"]["isError"])
            self.call("file_write_atomic", {"path": str(source), "content": "new", "overwrite": True})
            self.assertEqual(source.read_text(), "new")

    def test_archive_rejects_parent_traversal(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "bad.zip"
            with zipfile.ZipFile(archive, "w") as handle:
                handle.writestr("../outside.txt", "bad")
            result = self.request("tools/call", {"name": "archive_extract",
                                                 "arguments": {"archive": str(archive),
                                                               "destination": str(root / "out")}})
            self.assertTrue(result["result"]["isError"])
            self.assertFalse((root / "outside.txt").exists())

    def test_host_processes(self):
        run = self.call("process_run", {"executable": sys.executable,
                                        "arguments": ["-c", "print('hello from child')"],
                                        "timeoutMs": 5000})
        self.assertEqual(run["exitCode"], 0)
        self.assertIn("hello from child", run["stdout"])
        timed = self.call("process_run", {"executable": sys.executable,
                                          "arguments": ["-c", "import time; time.sleep(2)"],
                                          "timeoutMs": 100})
        self.assertTrue(timed["timedOut"])
        current = self.call("process_get", {"pid": os.getpid()})
        self.assertEqual(current["pid"], os.getpid())
        started = self.call("process_start", {"executable": sys.executable,
                                              "arguments": ["-c", "import time; print('ready', flush=True); time.sleep(.2)"]})
        self.assertGreater(started["pid"], 0)
        waited = self.call("process_wait", {"pid": started["pid"], "timeoutMs": 5000})
        self.assertEqual(waited["exitCode"], 0)
        self.assertIn("ready", self.call("process_output", {"pid": started["pid"],
                                                            "release": True})["stdout"])
        listed = self.call("process_list", {"limit": 1000})["processes"]
        self.assertTrue(any(item["pid"] == os.getpid() for item in listed))
        tree = self.call("process_tree", {"pid": os.getpid()})["processes"]
        self.assertTrue(any(item["pid"] == os.getpid() for item in tree))

    def test_interactive_process_and_output_cursor(self):
        child = self.call("process_start", {"executable": sys.executable,
                                            "arguments": ["-c", "import sys; print(sys.stdin.readline().upper(), end='')"]})
        self.assertEqual(self.call("process_input", {"pid": child["pid"], "data": "hello\n",
                                                     "close": True})["bytesWritten"], 6)
        self.assertEqual(self.call("process_wait", {"pid": child["pid"], "timeoutMs": 5000})["exitCode"], 0)
        output = self.call("process_output", {"pid": child["pid"], "stdoutOffset": 2,
                                               "release": True})
        self.assertEqual(output["stdout"].replace("\r\n", "\n"), "LLO\n")
        self.assertEqual(output["nextStdoutOffset"], 2 + len(output["stdout"].encode()))

    def test_wait_file_patch_and_http(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "sample.txt"
            path.write_text("alpha alpha")
            self.assertTrue(self.call("wait_for", {"kind": "file_exists", "path": str(path),
                                                    "timeoutMs": 1000})["ready"])
            self.assertFalse(self.call("wait_for", {"kind": "file_exists", "path": str(path) + ".missing",
                                                     "timeoutMs": 100, "intervalMs": 50})["ready"])
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            patched = self.call("file_patch", {"path": str(path), "find": "alpha", "replace": "beta",
                                                "expectedMatches": 2, "expectedSha256": digest})
            self.assertEqual(patched["matches"], 2)
            self.assertEqual(path.read_text(), "beta beta")
            rejected = self.request("tools/call", {"name": "file_patch", "arguments": {
                "path": str(path), "find": "beta", "replace": "gamma", "expectedSha256": digest}})
            self.assertTrue(rejected["result"]["isError"])
            self.assertEqual(path.read_text(), "beta beta")

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(404 if self.path == "/missing" else 200)
                self.send_header("Content-Type", "text/plain")
                self.end_headers()
                self.wfile.write(b"ready")

            def do_POST(self):
                body = self.rfile.read(int(self.headers["Content-Length"]))
                self.send_response(201)
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *_args):
                pass

        server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            base_url = f"http://127.0.0.1:{server.server_port}"
            url = base_url + "/health"
            response = self.call("http_request", {"url": url, "timeoutMs": 3000})
            self.assertEqual(response["status"], 200)
            self.assertEqual(response["body"], "ready")
            self.assertEqual(self.call("http_request", {"url": base_url + "/missing"})["status"], 404)
            self.assertEqual(self.call("http_request", {"url": url, "method": "POST",
                                                         "body": "payload"})["body"], "payload")
            self.assertTrue(self.call("wait_for", {"kind": "http", "url": url,
                                                    "expectedStatus": 200, "timeoutMs": 3000})["ready"])
            self.assertTrue(self.call("wait_for", {"kind": "tcp", "host": "127.0.0.1",
                                                    "port": server.server_port, "timeoutMs": 3000})["ready"])
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=3)

    def test_network_and_file_inspection(self):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            probe = self.call("network_probe", {"host": "127.0.0.1", "port": listener.getsockname()[1]})
            self.assertTrue(probe["connected"])
        resolved = self.call("dns_query", {"name": "localhost", "recordType": "A"})
        self.assertTrue(resolved["records"])
        executable = self.call("executable_resolve", {"name": sys.executable})
        self.assertTrue(Path(executable["path"]).is_file())
        self.assertIn(executable["architecture"], {"x86", "x86_64", "aarch64", "arm"})
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.txt").write_text("a")
            (root / "b.txt").write_text("b")
            manifest = self.call("directory_manifest", {"root": str(root), "hashFiles": True})
            self.assertEqual(manifest["fileCount"], 2)
            self.assertEqual(len(manifest["entries"][0]["sha256"]), 64)

    @unittest.skipUnless(os.name == "nt", "Windows-only signature inspection")
    def test_file_signature(self):
        signature = self.call("file_signature", {"path": sys.executable})
        self.assertIn("status", signature)
        self.assertIn("fileVersion", signature)

    def test_performance_sample(self):
        sample = self.call("performance_sample", {"sampleMs": 200})
        self.assertGreater(sample["totalMemoryBytes"], 0)
        self.assertIn("cpuPercent", sample)
        self.assertIsInstance(sample["networks"], list)
        self.assertIsInstance(sample["disks"], list)

    @unittest.skipUnless(os.name == "nt", "Windows-only diagnostics")
    def test_windows_diagnostics(self):
        tasks = self.call("scheduled_task", {"operation": "list", "limit": 5})
        self.assertIsInstance(tasks["tasks"], list)
        invalid = self.request("tools/call", {"name": "scheduled_task",
                                                "arguments": {"operation": "invalid"}})
        self.assertTrue(invalid["result"]["isError"])
        acl = self.call("acl_get", {"path": sys.executable})
        self.assertTrue(acl["owner"])
        events = self.call("eventlog_follow", {"logName": "System", "maxEvents": 2})
        self.assertIsInstance(events["events"], list)
        self.assertIn("nextRecordId", events)
        if events["nextRecordId"] is not None:
            newer = self.call("eventlog_follow", {"logName": "System", "maxEvents": 2,
                                                  "afterRecordId": events["nextRecordId"]})
            self.assertTrue(all(item["recordId"] > events["nextRecordId"] for item in newer["events"]))

    @unittest.skipUnless(os.name == "nt", "Windows-only operation")
    def test_process_stop(self):
        started = self.call("process_start", {"executable": sys.executable,
                                              "arguments": ["-c", "import time; time.sleep(30)"]})
        denied = self.request("tools/call", {"name": "process_stop",
                                                  "arguments": {"pid": started["pid"],
                                                                "expectedStartTimeUnix": 0}})
        self.assertTrue(denied["result"]["isError"])
        self.call("process_stop", {"pid": started["pid"]})
        self.assertTrue(self.call("process_wait", {"pid": started["pid"], "timeoutMs": 5000})["exited"])

    @unittest.skipUnless(os.name == "nt", "Windows-only operations")
    def test_windows_system_info_and_powershell(self):
        info = self.call("system_info", {})
        self.assertGreater(info["totalMemoryBytes"], 0)
        result = self.call("powershell_run", {"script": "Write-Output 42", "timeoutMs": 5000})
        self.assertEqual(result["stdout"].strip(), "42")
        self.assertEqual(self.call("environment_get", {"name": "PATH", "scope": "process"})["name"], "PATH")
        self.assertEqual(self.call("environment_get", {"name": "PATH", "scope": "user"})["scope"], "user")
        self.assertEqual(self.call("service_get", {"name": "EventLog"})["name"], "EventLog")
        bad_control = self.request("tools/call", {"name": "service_control",
                                                  "arguments": {"name": "EventLog", "action": "invalid"}})
        self.assertTrue(bad_control["result"]["isError"])
        self.assertIn("value", self.call("registry_read", {"path": r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                                                           "property": "ProductName"}))
        self.assertIsInstance(self.call("process_modules", {"pid": os.getpid()})["modules"], list)
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            port = listener.getsockname()[1]
            endpoints = self.call("port_owner", {"port": port})["endpoints"]
            self.assertTrue(any(item["pid"] == os.getpid() for item in endpoints))
        self.assertIsInstance(self.call("eventlog_query", {"logName": "System", "maxEvents": 1})["events"], list)

    def test_integer_and_bitfields(self):
        converted = self.call("integer_convert", {"value": "-1", "bits": 16})
        self.assertEqual(converted["hex"], "0xFFFF")
        self.assertEqual(converted["unsigned"], "65535")
        self.assertEqual(converted["littleEndianBytes"], "FF FF")
        wide = self.call("integer_convert", {"value": "0xFFFFFFFFFFFFFFFF", "bits": 64})
        self.assertEqual(wide["unsigned"], "18446744073709551615")
        self.assertEqual(wide["signed"], "-1")
        self.assertEqual(wide["binary"], "0b" + "1" * 64)
        extracted = self.call("bitfield_extract", {"value": "0xF0", "offset": 4, "width": 4})
        self.assertEqual(extracted["unsigned"], "15")
        inserted = self.call("bitfield_insert", {"value": "0xF0", "field": "0x3", "offset": 4,
                                                  "width": 4, "bits": 8})
        self.assertEqual(inserted["hex"], "0x30")

    def test_original_math_operations(self):
        self.assertEqual(self.call("add", {"firstNumber": 2, "secondNumber": 3})["value"], 5)
        self.assertEqual(self.call("subtract", {"minuend": 8, "subtrahend": 3})["value"], 5)
        self.assertEqual(self.call("multiply", {"firstNumber": 2, "secondNumber": 3})["value"], 6)
        self.assertEqual(self.call("division", {"numerator": 8, "denominator": 2})["value"], 4)
        self.assertEqual(self.call("sum", {"numbers": [1, 2, 3]})["value"], 6)
        self.assertEqual(self.call("modulo", {"numerator": 8, "denominator": 3})["value"], 2)
        self.assertEqual(self.call("floor", {"number": 1.7})["value"], 1)
        self.assertEqual(self.call("ceiling", {"number": 1.2})["value"], 2)
        self.assertEqual(self.call("round", {"number": 1.5})["value"], 2)
        self.assertEqual(self.call("mean", {"numbers": [1, 2, 3]})["value"], 2)
        self.assertEqual(self.call("median", {"numbers": [3, 1, 2]})["value"], 2)
        self.assertEqual(self.call("mode", {"numbers": [3, 1, 3]})["value"], 3)
        self.assertEqual(self.call("min", {"numbers": [3, 1, 2]})["value"], 1)
        self.assertEqual(self.call("max", {"numbers": [3, 1, 2]})["value"], 3)
        self.assertAlmostEqual(self.call("sin", {"number": 0})["value"], 0)
        self.assertAlmostEqual(self.call("arcsin", {"number": 1})["value"], 1.5707963267948966)
        self.assertAlmostEqual(self.call("cos", {"number": 0})["value"], 1)
        self.assertAlmostEqual(self.call("arccos", {"number": 1})["value"], 0)
        self.assertAlmostEqual(self.call("tan", {"number": 0})["value"], 0)
        self.assertAlmostEqual(self.call("arctan", {"number": 1})["value"], 0.7853981633974483)
        self.assertAlmostEqual(self.call("radiansToDegrees", {"number": 3.141592653589793})["value"], 180)
        self.assertAlmostEqual(self.call("degreesToRadians", {"number": 180})["value"], 3.141592653589793)

    def test_addresses_and_float_bits(self):
        address = self.call("address_translate", {"address": "0x140001234",
                                                   "imageBase": "0x140000000", "runtimeBase": "0x7FF600000000"})
        self.assertEqual(address["rva"], "0x1234")
        self.assertEqual(address["runtimeAddress"], "0x7FF600001234")
        relative = self.call("relative_target", {"instructionAddress": "0x1000",
                                                 "instructionSize": 5, "displacement": "-5"})
        self.assertEqual(relative["target"], "0x1000")
        value = self.call("ieee754_decode", {"bits": 32, "pattern": "0x3F800000"})
        self.assertEqual(value["classification"], "normal")
        self.assertEqual(value["value"], "1")
        aligned = self.call("align_address", {"address": "0x1003", "alignment": "0x10"})
        self.assertEqual(aligned["down"], "0x1000")
        self.assertEqual(aligned["up"], "0x1010")
        self.assertEqual(aligned["padding"], "13")

    def test_geometry(self):
        dot = self.call("vector_calculate", {"operation": "dot", "a": [1, 2, 3], "b": [4, 5, 6]})
        self.assertEqual(dot["value"], 32)
        cross = self.call("vector_calculate", {"operation": "cross", "a": [1, 0, 0], "b": [0, 1, 0]})
        self.assertEqual(cross["vector"], [0, 0, 1])
        bary = self.call("barycentric_2d", {"point": [0.25, 0.25], "a": [0, 0], "b": [1, 0], "c": [0, 1]})
        self.assertEqual(bary["weights"], [0.5, 0.25, 0.25])
        self.assertTrue(bary["inside"])
        ray = self.call("ray_triangle_intersect", {"origin": [0.25, 0.25, 1], "direction": [0, 0, -1],
                                                   "a": [0, 0, 0], "b": [1, 0, 0], "c": [0, 1, 0]})
        self.assertTrue(ray["hit"])
        self.assertEqual(ray["point"], [0.25, 0.25, 0])
        nonunit_ray = self.call("ray_triangle_intersect", {"origin": [0.25, 0.25, 1], "direction": [0, 0, -2],
                                                          "a": [0, 0, 0], "b": [1, 0, 0], "c": [0, 1, 0]})
        self.assertEqual(nonunit_ray["distance"], 1)
        tiny = self.call("vector_calculate", {"operation": "normalize", "a": [1e-20, 0]})
        self.assertEqual(tiny["vector"], [1, 0])
        huge_angle = self.call("vector_calculate", {"operation": "angle", "a": [1e308, 0],
                                                    "b": [1e308, 0]})
        self.assertEqual(huge_angle["value"], 0)
        huge_unit = self.call("vector_calculate", {"operation": "normalize", "a": [1e308, 1e308, 1e308]})
        self.assertAlmostEqual(sum(x*x for x in huge_unit["vector"]), 1)

    def test_transforms(self):
        identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]
        result = self.call("matrix_transform", {"matrix": identity, "vector": [2, 3, 4], "kind": "point"})
        self.assertEqual(result["vector"], [2, 3, 4])
        inverse = self.call("matrix_inverse", {"matrix": identity})
        self.assertEqual(inverse["matrix"], identity)
        translation = [1, 0, 0, 5, 0, 1, 0, 6, 0, 0, 1, 7, 0, 0, 0, 1]
        composition = self.call("matrix_multiply", {"a": translation, "b": identity})
        self.assertEqual(composition["matrix"], translation)
        translated = self.call("matrix_transform", {"matrix": translation, "vector": [2, 3, 4], "kind": "point"})
        self.assertEqual(translated["vector"], [7, 9, 11])
        tiny_scale = [1e-14, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]
        inverse_scale = self.call("matrix_inverse", {"matrix": tiny_scale})
        self.assertAlmostEqual(inverse_scale["matrix"][0], 1e14)
        rotated = self.call("quaternion_rotate", {"quaternion": [0, 0, 0, 1], "vector": [2, 3, 4]})
        self.assertEqual(rotated["vector"], [2, 3, 4])

    def test_invalid_input_is_tool_error(self):
        response = self.request("tools/call", {"name": "vector_calculate",
                                                "arguments": {"operation": "normalize", "a": [0, 0, 0]}})
        self.assertTrue(response["result"]["isError"])
        response = self.request("tools/call", {"name": "integer_convert", "arguments": {"value": "nope", "bits": 8}})
        self.assertTrue(response["result"]["isError"])
        response = self.request("tools/call", {"name": "division", "arguments": {"numerator": 1, "denominator": 0}})
        self.assertTrue(response["result"]["isError"])
        response = self.request("tools/call", {"name": "matrix_transform", "arguments": {
            "matrix": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1e308, 0, 0, 1],
            "vector": [1e308, 0, 0], "kind": "point"}})
        self.assertTrue(response["result"]["isError"])
        response = self.request("tools/call", {"name": "integer_convert", "arguments": {"value": "\u03c0", "bits": 8}})
        self.assertTrue(response["result"]["isError"])

    def test_protocol_notifications_and_errors(self):
        self.process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        self.process.stdin.flush()
        self.assertEqual(self.request("ping")["result"], {})
        self.process.stdin.write('{invalid json}\n')
        self.process.stdin.flush()
        invalid = json.loads(self.process.stdout.readline())
        self.assertIsNone(invalid["id"])
        self.assertEqual(invalid["error"]["code"], -32700)
        unknown = self.request("unknown/method")
        self.assertEqual(unknown["error"]["code"], -32601)

    def test_new_reverse_engineering_tools(self):
        sections = [{"virtualAddress": "0x1000", "virtualSize": "0x600",
                     "pointerToRawData": "0x400", "sizeOfRawData": "0x600"}]
        mapped = self.call("pe_address_map", {"mode": "rva_to_file", "value": "0x1100",
                                              "imageBase": "0x400000", "sizeOfHeaders": "0x400", "sections": sections})
        self.assertEqual(mapped["fileOffset"], "0x500")
        unpacked = self.call("binary_unpack", {"data": "00 00 80 3F", "type": "f32",
                                                "endian": "little", "offset": 0, "count": 1})
        self.assertEqual(unpacked["values"], [1.0])
        packed = self.call("binary_pack", {"values": ["0x1234"], "type": "u16", "endian": "little"})
        self.assertEqual(packed["data"], "34 12")
        self.assertEqual(self.call("bitwise_word", {"operation": "rotate_left", "a": "0x81",
                                                     "shift": 1, "bits": 8})["hex"], "0x03")
        alu = self.call("fixed_width_alu", {"operation": "add", "a": "0xFF", "b": "1", "bits": 8})
        self.assertEqual(alu["unsigned"], "0")
        self.assertTrue(alu["carry"])
        self.assertTrue(alu["zero"])
        leb = self.call("leb128_codec", {"mode": "encode", "signed": False, "value": "624485"})
        self.assertEqual(leb["data"], "E5 8E 26")
        self.assertEqual(self.call("leb128_codec", {"mode": "decode", "signed": False,
                                                     "data": "E5 8E 26"})["value"], "624485")
        self.assertEqual(self.call("x86_effective_address", {"base": "0x1000", "index": "3",
                                                              "scale": 4, "displacement": "-8", "bits": 64})["hex"], "0x1004")
        self.assertEqual(self.call("pe_relocation_apply", {"value": "0x401000", "oldBase": "0x400000",
                                                            "newBase": "0x500000", "type": "HIGHLOW"})["hex"], "0x501000")
        vertex = self.call("packed_vertex_decode", {"format": "R10G10B10A2_UNORM", "pattern": "0xC00003FF"})
        self.assertEqual(vertex["components"], [1, 0, 0, 1])
        crc = self.call("crc_compute", {"data": "31 32 33 34 35 36 37 38 39", "width": 32,
                                        "polynomial": "0x04C11DB7", "init": "0xFFFFFFFF",
                                        "xorOut": "0xFFFFFFFF", "reflectIn": True, "reflectOut": True})
        self.assertEqual(crc["hex"], "0xCBF43926")

    def test_new_geometry_tools(self):
        transform = self.call("compose_transform", {"translation": [5, 6, 7],
                                                    "rotation": [0, 0, 0, 1], "scale": [2, 3, 4]})["matrix"]
        self.assertEqual([transform[i] for i in (0, 5, 10, 3, 7, 11)], [2, 3, 4, 5, 6, 7])
        decomposed = self.call("decompose_transform", {"matrix": transform})
        self.assertEqual(decomposed["translation"], [5, 6, 7])
        self.assertEqual(decomposed["scale"], [2, 3, 4])
        rotation = self.call("rotation_convert", {"from": "axis_angle", "to": "quaternion",
                                                  "value": [0, 0, 1, 1.5707963267948966]})["value"]
        self.assertAlmostEqual(rotation[2], 0.7071067811865476)
        middle = self.call("quaternion_slerp", {"a": [0, 0, 0, 1], "b": [0, 0, 1, 0], "t": 0.5})
        self.assertAlmostEqual(middle["quaternion"][2], 0.7071067811865476)
        identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]
        projected = self.call("project_unproject", {"mode": "project", "matrix": identity,
                                                   "viewport": [0, 0, 100, 100], "vector": [0, 0, 0.5],
                                                   "depthRange": "zero_to_one"})
        self.assertEqual(projected["vector"], [50, 50, 0.5])
        unprojected = self.call("project_unproject", {"mode": "unproject", "matrix": identity,
                                                     "viewport": [0, 0, 100, 100], "vector": [50, 50, 0.5],
                                                     "depthRange": "zero_to_one"})
        self.assertEqual(unprojected["vector"], [0, 0, 0.5])
        hit = self.call("ray_primitive_intersect", {"primitive": "sphere", "origin": [0, 0, -3],
                                                    "direction": [0, 0, 1], "center": [0, 0, 0], "radius": 1})
        self.assertEqual(hit["distance"], 2)
        nearest = self.call("closest_point", {"primitive": "segment", "point": [2, 2, 0],
                                              "a": [0, 0, 0], "b": [2, 0, 0]})
        self.assertEqual(nearest["point"], [2, 0, 0])
        crossing = self.call("segment_intersect_2d", {"p1": [0, 0], "p2": [2, 2],
                                                      "q1": [0, 2], "q2": [2, 0]})
        self.assertEqual(crossing["point"], [1, 1])
        polygon = self.call("polygon_measure_2d", {"points": [[0, 0], [2, 0], [2, 2], [0, 2]]})
        self.assertEqual(polygon["area"], 4)
        self.assertEqual(polygon["centroid"], [1, 1])
        frustum = self.call("frustum_test", {"matrix": identity, "primitive": "point",
                                             "point": [0, 0, 0.5], "depthRange": "zero_to_one"})
        self.assertEqual(frustum["classification"], "inside")

    def test_new_arithmetic_tools(self):
        self.assertEqual(self.call("power_root_log", {"operation": "pow", "a": 2, "b": 10})["value"], 1024)
        fraction = self.call("exact_fraction", {"operation": "add", "a": {"numerator": "1", "denominator": "3"},
                                                 "b": {"numerator": "1", "denominator": "6"}})
        self.assertEqual((fraction["numerator"], fraction["denominator"]), ("1", "2"))
        self.assertEqual(self.call("gcd_extended", {"a": "240", "b": "46"})["gcd"], "2")
        self.assertEqual(self.call("modular_arithmetic", {"operation": "pow", "a": "4", "b": "13",
                                                          "modulus": "497"})["value"], "445")
        self.assertEqual(self.call("combinatorics_exact", {"operation": "combination", "n": 52,
                                                           "k": 5})["value"], "2598960")
        complex_value = self.call("complex_calculate", {"operation": "multiply", "a": [1, 2], "b": [3, 4]})
        self.assertEqual((complex_value["real"], complex_value["imag"]), (-5, 10))
        polynomial = self.call("polynomial_evaluate", {"coefficients": [2, 3, 1], "x": 2})
        self.assertEqual((polynomial["value"], polynomial["derivative"]), (15, 11))
        system = self.call("linear_system_solve", {"matrix": [[2, 1], [1, -1]], "b": [5, 1]})
        self.assertEqual(system["solution"], [2, 1])
        stats = self.call("descriptive_statistics", {"numbers": [1, 2, 3], "sample": False})
        self.assertEqual(stats["mean"], 2)
        self.assertAlmostEqual(stats["variance"], 2/3)
        self.assertEqual(self.call("interpolate_samples", {"mode": "linear", "p1": 10, "p2": 20,
                                                             "t": 0.25})["value"], 12.5)

    def test_new_tool_edge_cases(self):
        plane = self.call("ray_primitive_intersect", {"primitive": "plane", "origin": [0, 0, 0],
                                                      "direction": [0, 0, 1], "normal": [0, 0, 2],
                                                      "distance": -4})
        self.assertEqual(plane["distance"], 2)
        self.assertEqual(self.call("leb128_codec", {"mode": "encode", "signed": True, "value": "-2"})["data"], "7E")
        self.assertEqual(self.call("leb128_codec", {"mode": "decode", "signed": True, "data": "7E"})["value"], "-2")
        self.assertEqual(self.call("binary_pack", {"values": [1.0], "type": "f16", "endian": "little"})["data"], "00 3C")
        self.assertEqual(self.call("binary_unpack", {"data": "FF FF FF FF FF FF FF FF", "type": "i64",
                                                      "endian": "big", "offset": 0, "count": 1})["values"], ["-1"])
        crc = self.call("crc_compute", {"data": "31 32 33 34 35 36 37 38 39", "width": 16,
                                        "polynomial": "0x1021", "init": "0", "xorOut": "0",
                                        "reflectIn": False, "reflectOut": False})
        self.assertEqual(crc["hex"], "0x31C3")
        overflow = self.call("fixed_width_alu", {"operation": "add", "a": "0x7F", "b": "1", "bits": 8})
        self.assertTrue(overflow["overflow"])
        self.assertEqual(self.call("modular_arithmetic", {"operation": "inverse", "a": "3",
                                                          "modulus": "11"})["value"], "4")
        overlap = self.call("segment_intersect_2d", {"p1": [0, 0], "p2": [3, 0],
                                                     "q1": [1, 0], "q2": [2, 0]})
        self.assertEqual(overlap["kind"], "overlap")
        corr = self.call("descriptive_statistics", {"numbers": [1, 2, 3], "other": [3, 2, 1]})
        self.assertAlmostEqual(corr["correlation"], -1)
        divided = self.call("complex_calculate", {"operation": "divide", "a": [1e308, 0], "b": [1e308, 0]})
        self.assertEqual((divided["real"], divided["imag"]), (1, 0))
        too_large = self.request("tools/call", {"name": "binary_pack", "arguments": {
            "values": [1e100], "type": "f16", "endian": "little"}})
        self.assertTrue(too_large["result"]["isError"])
        not_rotation = self.request("tools/call", {"name": "rotation_convert", "arguments": {
            "from": "matrix", "to": "quaternion", "value": [2, 0, 0, 0, 1, 0, 0, 0, 1]}})
        self.assertTrue(not_rotation["result"]["isError"])
        pe_overflow = self.request("tools/call", {"name": "pe_address_map", "arguments": {
            "mode": "rva_to_file", "value": "0x1300", "imageBase": "0",
            "sizeOfHeaders": "0x400", "sections": [{"virtualAddress": "0x1000",
                "virtualSize": "0x400", "pointerToRawData": "0xFFFFFFFFFFFFFF00",
                "sizeOfRawData": "0x400"}]}})
        self.assertTrue(pe_overflow["result"]["isError"])
        frustum_overflow = self.request("tools/call", {"name": "frustum_test", "arguments": {
            "matrix": [1, 0, 0, 1e308, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1e308],
            "primitive": "point", "point": [0, 0, 0.5], "depthRange": "zero_to_one"}})
        self.assertTrue(frustum_overflow["result"]["isError"])

    def test_new_modes_and_roundtrips(self):
        sections = [{"virtualAddress": "0x1000", "virtualSize": "0x600",
                     "pointerToRawData": "0x400", "sizeOfRawData": "0x400"}]
        file_to_va = self.call("pe_address_map", {"mode": "file_to_va", "value": "0x500",
                                                  "imageBase": "0x400000", "sizeOfHeaders": "0x400", "sections": sections})
        self.assertEqual(file_to_va["virtualAddress"], "0x401100")
        zero_fill = self.request("tools/call", {"name": "pe_address_map", "arguments": {
            "mode": "rva_to_file", "value": "0x1500", "imageBase": "0x400000",
            "sizeOfHeaders": "0x400", "sections": sections}})
        self.assertTrue(zero_fill["result"]["isError"])
        self.assertEqual(self.call("bitwise_word", {"operation": "shift_right", "a": "-1",
                                                     "shift": 1, "bits": 8})["hex"], "0x7F")
        self.assertEqual(self.call("pe_relocation_apply", {"value": "0x140001000", "oldBase": "0x140000000",
                                                            "newBase": "0x180000000", "type": "DIR64"})["hex"], "0x180001000")
        reflected = self.call("packed_vertex_decode", {"format": "R8G8B8A8_SNORM", "pattern": "0x00000080"})
        self.assertEqual(reflected["components"][0], -1)
        negative_scale = self.call("compose_transform", {"translation": [0, 0, 0],
                                                         "rotation": [0, 0, 0, 1], "scale": [-2, 3, 4]})["matrix"]
        decomposed = self.call("decompose_transform", {"matrix": negative_scale})
        self.assertEqual(decomposed["scale"], [-2, 3, 4])
        self.assertTrue(decomposed["reflection"])
        aabb = self.call("ray_primitive_intersect", {"primitive": "aabb", "origin": [0, 0, -3],
                                                     "direction": [0, 0, 1], "min": [-1, -1, -1], "max": [1, 1, 1]})
        self.assertEqual(aabb["distance"], 2)
        nearest = self.call("closest_point", {"primitive": "triangle", "point": [0.25, 0.25, 2],
                                              "a": [0, 0, 0], "b": [1, 0, 0], "c": [0, 1, 0]})
        self.assertEqual(nearest["point"], [0.25, 0.25, 0])
        outside = self.call("frustum_test", {"matrix": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
                                             "primitive": "sphere", "center": [5, 0, 0], "radius": 0.5,
                                             "depthRange": "zero_to_one"})
        self.assertEqual(outside["classification"], "outside")
        self.assertEqual(self.call("exact_fraction", {"operation": "compare",
                                                       "a": {"numerator": "2", "denominator": "3"},
                                                       "b": {"numerator": "3", "denominator": "4"}})["comparison"], -1)
        self.assertEqual(self.call("combinatorics_exact", {"operation": "permutation", "n": 5, "k": 2})["value"], "20")
        self.assertEqual(self.call("interpolate_samples", {"mode": "hermite", "p1": 0, "p2": 1,
                                                             "tangent1": 0, "tangent2": 0, "t": 0.5})["value"], 0.5)

    def test_vector_operators_by_dimension(self):
        self.assertEqual(self.call("vector_n", {"operation": "add", "a": [1, 2, 3, 4],
                                                  "b": [5, 6, 7, 8]})["vector"], [6, 8, 10, 12])
        self.assertEqual(self.call("vector_n", {"operation": "scale", "a": [1, 2],
                                                  "scalar": 3})["vector"], [3, 6])
        self.assertEqual(self.call("vector_n", {"operation": "project", "a": [2, 3, 0],
                                                  "b": [1, 0, 0]})["vector"], [2, 0, 0])
        self.assertEqual(self.call("vector_n", {"operation": "reject", "a": [2, 3, 0],
                                                  "b": [1, 0, 0]})["vector"], [0, 3, 0])
        self.assertEqual(self.call("vector_n", {"operation": "reflect", "a": [1, -1],
                                                  "b": [0, 1]})["vector"], [1, 1])
        self.assertEqual(self.call("vector_n", {"operation": "clamp", "a": [-1, 5],
                                                  "lower": [0, 0], "upper": [1, 4]})["vector"], [0, 4])
        self.assertEqual(self.call("vector_n", {"operation": "lerp", "a": [0, 10],
                                                  "b": [10, 20], "t": 0.25})["vector"], [2.5, 12.5])
        self.assertEqual(self.call("vector_special", {"operation": "perp", "a": [2, 3]})["vector"], [-3, 2])
        self.assertAlmostEqual(self.call("vector_special", {"operation": "signed_angle",
                                                            "a": [1, 0], "b": [0, 1]})["value"], 1.5707963267948966)
        self.assertEqual(self.call("vector_special", {"operation": "scalar_triple", "a": [1, 0, 0],
                                                        "b": [0, 1, 0], "c": [0, 0, 1]})["value"], 1)
        self.assertEqual(self.call("vector_special", {"operation": "homogenize", "a": [1, 2, 3],
                                                        "w": 2})["vector"], [1, 2, 3, 2])
        self.assertEqual(self.call("vector_special", {"operation": "perspective_divide",
                                                        "a": [2, 4, 6, 2]})["vector"], [1, 2, 3])

    def test_rectangular_matrices_and_layout(self):
        a = {"rows": 2, "cols": 3, "data": [1, 2, 3, 4, 5, 6]}
        b = {"rows": 3, "cols": 2, "data": [7, 8, 9, 10, 11, 12]}
        product = self.call("matrix_n", {"operation": "multiply", "a": a, "b": b})["matrix"]
        self.assertEqual(product, {"rows": 2, "cols": 2, "data": [58, 64, 139, 154]})
        transposed = self.call("matrix_n", {"operation": "transpose", "a": a})["matrix"]
        self.assertEqual(transposed, {"rows": 3, "cols": 2, "data": [1, 4, 2, 5, 3, 6]})
        self.assertEqual(self.call("matrix_n", {"operation": "matvec", "a": a,
                                                  "vector": [1, 1, 1]})["vector"], [6, 15])
        outer = self.call("matrix_n", {"operation": "outer", "u": [1, 2], "v": [3, 4, 5]})["matrix"]
        self.assertEqual(outer, {"rows": 2, "cols": 3, "data": [3, 4, 5, 6, 8, 10]})
        converted = self.call("matrix_layout_convert", {"rows": 2, "cols": 3, "data": a["data"],
                                                        "sourceLayout": "row_major", "targetLayout": "column_major",
                                                        "targetStride": 3})
        self.assertEqual(converted["data"], [1, 4, 0, 2, 5, 0, 3, 6, 0])
        restored = self.call("matrix_layout_convert", {"rows": 2, "cols": 3, "data": converted["data"],
                                                       "sourceLayout": "column_major", "sourceStride": 3,
                                                       "targetLayout": "row_major"})
        self.assertEqual(restored["data"], a["data"])

    def test_matrix_properties_factors_and_affine(self):
        m = {"rows": 2, "cols": 2, "data": [4, 7, 2, 6]}
        self.assertEqual(self.call("matrix_properties", {"operation": "determinant", "matrix": m})["value"], 10)
        self.assertEqual(self.call("matrix_properties", {"operation": "trace", "matrix": m})["value"], 10)
        self.assertEqual(self.call("matrix_properties", {"operation": "rank", "matrix": m})["rank"], 2)
        inverse = self.call("matrix_properties", {"operation": "inverse", "matrix": m})["matrix"]["data"]
        for actual, expected in zip(inverse, [0.6, -0.7, -0.2, 0.4]):
            self.assertAlmostEqual(actual, expected)
        lu = self.call("matrix_factor", {"operation": "lu", "matrix": m})
        self.assertIn("l", lu)
        self.assertIn("u", lu)
        self.assertIn("permutation", lu)
        qr = self.call("matrix_factor", {"operation": "qr", "matrix": m})
        self.assertIn("q", qr)
        self.assertIn("r", qr)
        cholesky = self.call("matrix_factor", {"operation": "cholesky", "matrix": {
            "rows": 2, "cols": 2, "data": [4, 2, 2, 3]}})
        self.assertAlmostEqual(cholesky["l"]["data"][0], 2)
        svd = self.call("matrix_factor", {"operation": "svd", "matrix": m})
        self.assertEqual(len(svd["singularValues"]), 2)
        pinv = self.call("matrix_factor", {"operation": "pseudoinverse", "matrix": m})["matrix"]
        self.assertEqual((pinv["rows"], pinv["cols"]), (2, 2))
        least = self.call("matrix_factor", {"operation": "least_squares", "matrix": {
            "rows": 3, "cols": 2, "data": [1, 0, 0, 1, 1, 1]}, "rhs": [2, 3, 5]})
        for actual, expected in zip(least["solution"], [2, 3]):
            self.assertAlmostEqual(actual, expected)
        affine = {"rows": 2, "cols": 3, "data": [2, 0, 5, 0, 3, 6]}
        self.assertEqual(self.call("affine_transform", {"operation": "transform_point", "dimension": 2,
                                                        "matrix": affine, "vector": [1, 2]})["vector"], [7, 12])
        self.assertEqual(self.call("affine_transform", {"operation": "transform_direction", "dimension": 2,
                                                        "matrix": affine, "vector": [1, 2]})["vector"], [2, 6])
        self.assertEqual(self.call("affine_transform", {"operation": "transform_normal", "dimension": 2,
                                                        "matrix": affine, "vector": [1, 0]})["vector"], [1, 0])
        expanded = self.call("affine_transform", {"operation": "expand", "dimension": 2,
                                                       "matrix": affine})["matrix"]
        self.assertEqual(expanded, {"rows": 3, "cols": 3, "data": [2, 0, 5, 0, 3, 6, 0, 0, 1]})

    def test_new_linear_modes_and_factor_reconstruction(self):
        a = {"rows": 2, "cols": 2, "data": [0, 1, 1, 1]}
        lu = self.call("matrix_factor", {"operation": "lu", "matrix": a})
        left = self.call("matrix_n", {"operation": "multiply", "a": lu["permutation"], "b": a})["matrix"]["data"]
        right = self.call("matrix_n", {"operation": "multiply", "a": lu["l"], "b": lu["u"]})["matrix"]["data"]
        for x, y in zip(left, right):
            self.assertAlmostEqual(x, y)
        qr = self.call("matrix_factor", {"operation": "qr", "matrix": a})
        recomposed = self.call("matrix_n", {"operation": "multiply", "a": qr["q"], "b": qr["r"]})["matrix"]["data"]
        for x, y in zip(recomposed, a["data"]):
            self.assertAlmostEqual(x, y)
        singular = {"rows": 2, "cols": 2, "data": [1, 2, 2, 4]}
        self.assertEqual(self.call("matrix_properties", {"operation": "condition", "matrix": singular})["value"], "Infinity")
        self.assertEqual(self.call("matrix_properties", {"operation": "rank", "matrix": singular})["rank"], 1)
        self.assertEqual(self.call("matrix_n", {"operation": "hadamard", "a": a, "b": a})["matrix"]["data"], [0, 1, 1, 1])
        self.assertEqual(self.call("matrix_n", {"operation": "scale", "a": a, "scalar": 2})["matrix"]["data"], [0, 2, 2, 2])
        self.assertEqual(self.call("vector_n", {"operation": "hadamard", "a": [2, 3], "b": [4, 5]})["vector"], [8, 15])
        self.assertEqual(self.call("vector_n", {"operation": "divide", "a": [8, 15], "b": [4, 5]})["vector"], [2, 3])
        normal = self.call("affine_transform", {"operation": "transform_normal", "dimension": 2,
                                                "matrix": {"rows": 2, "cols": 3, "data": [2, 0, 0, 0, 1, 0]},
                                                "vector": [1, 1]})["vector"]
        self.assertAlmostEqual(normal[0], 0.4472135954999579)
        self.assertAlmostEqual(normal[1], 0.8944271909999159)
        batched = self.call("affine_transform", {"operation": "batch_points", "dimension": 2,
                                                 "matrix": {"rows": 2, "cols": 3, "data": [1, 0, 2, 0, 1, 3]},
                                                 "vectors": [[0, 0], [1, 1]]})
        self.assertEqual(batched["vectors"], [[2, 3], [3, 4]])

    def test_new_linear_argument_errors(self):
        cases = [
            ("vector_n", {"operation": "project", "a": [1, 2], "b": [0, 0]}),
            ("matrix_n", {"operation": "multiply", "a": {"rows": 2, "cols": 3, "data": [1, 2, 3, 4, 5, 6]},
                          "b": {"rows": 2, "cols": 2, "data": [1, 0, 0, 1]}}),
            ("matrix_properties", {"operation": "determinant", "matrix": {"rows": 2, "cols": 3,
                                   "data": [1, 2, 3, 4, 5, 6]}}),
            ("matrix_factor", {"operation": "cholesky", "matrix": {"rows": 2, "cols": 2,
                               "data": [1, 2, 3, 4]}}),
            ("affine_transform", {"operation": "transform_normal", "dimension": 2,
                                  "matrix": {"rows": 2, "cols": 3, "data": [0, 0, 0, 0, 1, 0]},
                                  "vector": [1, 0]}),
            ("matrix_layout_convert", {"rows": 2, "cols": 3, "data": [1, 2, 3, 4, 5, 6],
                                       "sourceLayout": "row_major", "targetLayout": "column_major",
                                       "sourceStride": 2}),
        ]
        for name, arguments in cases:
            with self.subTest(name=name):
                response = self.request("tools/call", {"name": name, "arguments": arguments})
                self.assertTrue(response["result"]["isError"])


if __name__ == "__main__":
    unittest.main()
