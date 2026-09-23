# EveryMCP tool guide

EveryMCP exposes 124 tools. This guide gives each tool's purpose and one practical use case. Call `tools/list` for the authoritative JSON schema, supported options, and current descriptions.

A tool call uses the standard MCP envelope:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"add","arguments":{"firstNumber":2,"secondNumber":3}}}
```

For exact integer and binary operations, pass integer values as strings when the schema requests them. Vectors are numeric arrays. Matrix tools use row-major storage unless their description says otherwise. Local host tools run with the permissions of the account that starts the server. Windows-only tools are identified below.

## Basic arithmetic, trigonometry, and aggregates

| Tool | What to use it for | Example use case |
| --- | --- | --- |
| `add` | Add two numbers. | Combine two measured durations. |
| `subtract` | Subtract `subtrahend` from `minuend`. | Calculate the difference between expected and observed sizes. |
| `multiply` | Multiply two numbers. | Convert a unit price and quantity into a total. |
| `division` | Divide a numerator by a denominator. | Calculate bytes per record from a file size and record count. |
| `sum` | Add an array of numbers. | Total the durations of several pipeline stages. |
| `modulo` | Compute a remainder. | Wrap a cyclic buffer index. |
| `floor` | Round down to an integer. | Find the fully completed blocks in a partial transfer. |
| `ceiling` | Round up to an integer. | Determine how many fixed-size pages are needed. |
| `round` | Round to the nearest integer. | Display an estimated item count. |
| `sin` | Evaluate sine in radians. | Calculate a vertical component from an angle. |
| `arcsin` | Recover an angle from a sine value. | Estimate pitch from a normalized vertical component. |
| `cos` | Evaluate cosine in radians. | Calculate a horizontal component from an angle. |
| `arccos` | Recover an angle from a cosine value. | Convert a normalized dot product into an angle. |
| `tan` | Evaluate tangent in radians. | Compute a slope from its inclination. |
| `arctan` | Recover an angle from a tangent value. | Convert a measured slope into an inclination. |
| `radiansToDegrees` | Convert radians to degrees. | Present a computed rotation in a UI. |
| `degreesToRadians` | Convert degrees to radians. | Feed a user-supplied angle into a trig operation. |
| `mean` | Calculate an arithmetic average. | Summarize repeated benchmark timings. |
| `median` | Find the middle value. | Report a typical latency without letting one spike dominate. |
| `mode` | Find the most frequent value. | Identify the most common status code in a sample. |
| `min` | Find the smallest number. | Locate the lowest sensor reading. |
| `max` | Find the largest number. | Find a peak memory sample. |

## Integers, binary formats, and reverse engineering

| Tool | What to use it for | Example use case |
| --- | --- | --- |
| `integer_convert` | Interpret a fixed-width integer as signed or unsigned and show hex, binary, and endian bytes. | Inspect a 64-bit identifier without losing precision in JSON. |
| `bitfield_extract` | Read a bit range from a word. | Decode flags stored in a packed header. |
| `bitfield_insert` | Replace a bit range in an 8-, 16-, 32-, or 64-bit word. | Construct a header with a new field value. |
| `address_translate` | Convert an image VA into an RVA and a rebased runtime VA. | Correlate a static address with a loaded module. |
| `align_address` | Align an address up or down and report padding. | Place the next section on a required boundary. |
| `relative_target` | Resolve a PC-relative displacement from the end of an instruction. | Find the destination of a relative branch. |
| `ieee754_decode` | Interpret a 32- or 64-bit IEEE-754 bit pattern. | Check whether raw bytes encode a float, infinity, or NaN. |
| `pe_address_map` | Map among PE VA, RVA, and file offsets using section metadata. | Locate on-disk bytes for a function RVA. |
| `binary_unpack` | Decode typed values from hex bytes with endian and stride controls. | Read a sequence of 16-bit fields in a binary record. |
| `binary_pack` | Encode typed integers or floats as endian-aware bytes. | Build a small binary fixture for a parser test. |
| `bitwise_word` | Apply fixed-width bitwise, shift, or rotate operations. | Reproduce a mask and rotate step seen in machine code. |
| `fixed_width_alu` | Perform wrapped arithmetic and inspect CPU-style flags. | Verify overflow behavior of a 32-bit counter. |
| `leb128_codec` | Encode or decode signed or unsigned LEB128. | Inspect a variable-length integer in a byte stream. |
| `x86_effective_address` | Evaluate base plus scaled index and displacement with fixed-width wrap. | Reconstruct an x86 memory operand address. |
| `pe_relocation_apply` | Apply a PE HIGHLOW or DIR64 base relocation. | Predict a pointer after a module is rebased. |
| `packed_vertex_decode` | Decode supported packed vertex component formats. | Check position or color values stored in a compact GPU buffer. |
| `crc_compute` | Compute a CRC with explicit polynomial and reflection settings. | Match a checksum used by a custom binary format. |

## Geometry, rotations, and spatial tests

| Tool | What to use it for | Example use case |
| --- | --- | --- |
| `vector_calculate` | Compute dot, cross, length, normalize, distance, or angle for 2D/3D vectors. | Check whether two directions are nearly perpendicular. |
| `matrix_transform` | Transform a 3D point or direction with a row-major 4x4 matrix. | Compare a local point with its world-space position. |
| `matrix_multiply` | Compose two row-major 4x4 transforms. | Combine parent and child transforms. |
| `matrix_inverse` | Invert a nonsingular row-major 4x4 matrix. | Convert a world-space point back into local space. |
| `quaternion_rotate` | Rotate a 3D vector with an XYZW quaternion. | Find a camera's forward direction. |
| `barycentric_2d` | Calculate triangle weights and inside status. | Determine whether a cursor lies inside a 2D triangle. |
| `ray_triangle_intersect` | Test a 3D ray against a triangle. | Select a triangle under a pointer ray. |
| `compose_transform` | Build a T*R*S affine matrix from translation, rotation, and scale. | Create an object transform from editor fields. |
| `decompose_transform` | Extract translation, rotation, signed scale, and shear from an affine matrix. | Diagnose unexpected scale or reflection in an imported transform. |
| `rotation_convert` | Convert among quaternion, axis-angle, Euler XYZ, and 3x3 matrix forms. | Compare rotations produced by two APIs with different representations. |
| `quaternion_slerp` | Interpolate between rotations along the shortest arc. | Compute orientation midway through an animation. |
| `project_unproject` | Map points between 3D clip space and a top-left viewport. | Convert a screen click into a 3D point or inspect projected coordinates. |
| `ray_primitive_intersect` | Test a ray against a plane, sphere, or AABB. | Pick a bounding box in a 3D viewport. |
| `closest_point` | Find the closest point on a segment, triangle, or AABB. | Measure clearance from a point to a collision primitive. |
| `segment_intersect_2d` | Find the point or overlap of two closed 2D segments. | Detect crossing edges in a path. |
| `polygon_measure_2d` | Measure signed area, centroid, and winding. | Check polygon orientation before triangulation. |
| `frustum_test` | Classify a point, sphere, or AABB against a view frustum. | Decide whether a bound can be culled from a camera view. |

## Exact and numerical analysis

| Tool | What to use it for | Example use case |
| --- | --- | --- |
| `power_root_log` | Compute powers, roots, logarithms, or exponentials. | Compare exponential growth models. |
| `exact_fraction` | Perform exact fraction arithmetic and comparisons. | Avoid float rounding in rational unit conversions. |
| `gcd_extended` | Find GCD, LCM, and Bézout coefficients for integer strings. | Derive coefficients for a modular inverse proof. |
| `modular_arithmetic` | Add, multiply, exponentiate, or invert modulo a positive integer. | Check a rolling counter or modular algorithm. |
| `combinatorics_exact` | Compute exact factorials, permutations, or combinations. | Count possible selections without floating-point overflow. |
| `complex_calculate` | Perform complex arithmetic or polar conversion. | Work through a frequency-domain calculation. |
| `polynomial_evaluate` | Evaluate a polynomial and its derivative. | Sample a calibration curve and slope at one input. |
| `linear_system_solve` | Solve a square linear system and report residual error. | Fit coefficients from a small system of constraints. |
| `descriptive_statistics` | Compute spread, quartiles, and optional correlation. | Compare benchmark distributions from two runs. |
| `interpolate_samples` | Evaluate linear, Hermite, or Catmull-Rom scalar interpolation. | Sample a keyframe curve between stored points. |

## General vectors and matrices

| Tool | What to use it for | Example use case |
| --- | --- | --- |
| `vector_n` | Apply elementwise, interpolation, projection, rejection, or reflection operations to 2–32 components. | Reflect a velocity vector off a surface normal. |
| `vector_special` | Use dimension-specific perpendicular, signed-angle, triple-product, or homogeneous operations. | Convert a 4D homogeneous point to 3D. |
| `matrix_n` | Add, scale, transpose, multiply, or apply matrices up to 32x32. | Multiply a transform or small coefficient matrix by a vector. |
| `matrix_properties` | Inspect trace, determinant, rank, norms, condition, or inverse. | Detect whether a calibration matrix is singular or poorly conditioned. |
| `matrix_factor` | Compute LU, QR, Cholesky, SVD, pseudoinverse, or least squares. | Solve an overdetermined fit with least squares. |
| `affine_transform` | Apply or expand 2D/3D affine matrices to points, directions, normals, or batches. | Transform an entire list of vertices consistently. |
| `matrix_layout_convert` | Convert flat data between row-major and column-major layouts and strides. | Pass a matrix to a library with a different storage convention. |

## Artifacts, structured data, and workspaces

| Tool | What to use it for | Example use case |
| --- | --- | --- |
| `artifact_manifest` | Hash a directory into a stable snapshot; optionally save it as JSON. | Record the exact contents of a release package before publishing it. |
| `manifest_diff` | Compare directories or saved artifact snapshots by path, type, size, and SHA-256. | Identify which release files changed between two builds. |
| `diagnostics_parse` | Normalize build-log errors and warnings into structured locations and messages. | Extract actionable compiler errors from a long CMake or Rust log. |
| `test_results_parse` | Summarize JUnit XML, TRX, or TAP results and list failures. | Combine a CI test report into a short pass/fail summary. |
| `structured_data_query` | Read JSON, TOML, YAML, XML, or CSV using a JSON Pointer. | Retrieve one setting from a large configuration file. |
| `structured_data_diff` | Compare two structured files and report pointer-addressed changes. | Review which configuration values changed between releases. |
| `dependency_inventory` | List direct Cargo, Python, npm, or NuGet dependencies and available locked or local versions. | Spot a pinned dependency that differs from the local installation. |
| `workspace_scan` | Discover repositories and build manifests below a directory. | Find all projects inside a shared development folder. |
| `repo_status_batch` | Summarize branches, HEADs, changed files, and worktree counts for several repositories. | Check which projects have uncommitted work before updating them. |
| `command_pipeline` | Run explicit executable steps in order with bounded output and stop-on-failure behavior. | Run configure, build, and test commands as one operation. |
| `environment_snapshot` | Record OS, architecture, selected variables, and executable version probes. | Capture the toolchain used to reproduce a build failure. |
| `file_watch` | Wait for added, removed, or metadata-modified files in a directory. | Continue once a generator writes its output file. |

## Processes

| Tool | What to use it for | Example use case |
| --- | --- | --- |
| `process_start` | Launch an executable and keep bounded output for later reads. | Start a local server while running other checks. |
| `process_run` | Run an executable to completion with timeout and captured stdout/stderr. | Run a compiler and inspect its exit code and errors. |
| `process_get` | Inspect a PID, including sampled CPU, memory, command line, and I/O. | Check whether a running build is still consuming resources. |
| `process_list` | List processes, optionally filtering by name. | Find running instances of an application. |
| `process_tree` | Show a PID and its descendants. | Identify helpers spawned by a test runner. |
| `process_wait` | Wait for a managed child or any PID to exit. | Continue packaging after a build process finishes. |
| `process_output` | Read captured output incrementally from a managed child. | Follow a long-running server log without rereading earlier bytes. |
| `process_stop` | Stop a PID, optionally with its descendants and a start-time check. | End a stuck test process tree. |
| `process_modules` | List modules loaded by a Windows process. | Confirm which native DLL version an application loaded. |
| `process_input` | Send UTF-8 or hex data to a managed child's stdin, or close it. | Drive an interactive command-line program through its prompt. |

## Files, archives, and networking

| Tool | What to use it for | Example use case |
| --- | --- | --- |
| `file_find` | Find files by basename wildcard below a root. | Locate all generated `.json` reports in a build tree. |
| `text_search` | Search UTF-8 files for literal or regex matches with locations. | Find every reference to a renamed configuration key. |
| `file_stat` | Read a path's type, size, and modification time. | Check whether a build output exists and when it changed. |
| `file_hash` | Stream a file into a SHA-256 or SHA-512 digest. | Confirm that a downloaded binary matches an expected hash. |
| `file_read_range` | Read a bounded byte range as UTF-8 or hex. | Inspect a file header without loading the whole file. |
| `file_write_atomic` | Write a UTF-8 file through a temporary file and atomic replacement. | Save a generated configuration without exposing a partial write. |
| `file_copy_move` | Copy or move a file or directory. | Put a finished artifact in an output folder. |
| `archive_create` | Create a ZIP from files and directories. | Package build outputs for transfer. |
| `archive_extract` | Extract a ZIP with path-traversal protection. | Unpack a dependency archive into a staging directory. |
| `wait_for` | Wait for a file, process exit, TCP port, or HTTP status. | Wait until a local HTTP service is ready before testing it. |
| `file_patch` | Replace exact text or bytes atomically with match-count and optional hash preconditions. | Change a known value only if the source file is the expected revision. |
| `http_request` | Make a bounded HTTP(S) request with method, headers, and optional body. | Query a local service's health endpoint. |
| `network_probe` | Probe TCP or verified TLS connectivity and latency. | Check that a service accepts connections before sending a request. |
| `dns_query` | Query A, AAAA, CNAME, MX, NS, TXT, or SRV records. | Verify service-discovery records after a DNS change. |
| `executable_resolve` | Find an executable on `PATH` or by path and inspect available metadata. | Determine which compiler installation a command will use. |
| `directory_manifest` | Inventory a directory with optional per-file SHA-256 hashes. | Audit the contents of a deployment folder. |

## Host inspection and Windows administration

The Windows administration tools in this section require Windows. `system_info` and `performance_sample` are cross-platform; `environment_get` can read process scope anywhere, while user and machine scopes use Windows facilities.

| Tool | What to use it for | Example use case |
| --- | --- | --- |
| `port_owner` | Find Windows TCP/UDP endpoints and their owning PIDs. | Identify the process blocking a development port. |
| `service_get` | Inspect a Windows service's status, startup mode, and PID. | Check whether a database service started. |
| `service_control` | Start, stop, or restart a Windows service. | Restart a local service after replacing its configuration. |
| `eventlog_query` | Read recent Windows Event Log entries. | Inspect application errors after a failed launch. |
| `registry_read` | Read one property from a literal Windows Registry path. | Check an installed application's configured location. |
| `environment_get` | Read one process, user, or machine environment variable. | Check which SDK path a build will inherit. |
| `system_info` | Read OS, CPU, and memory information. | Record the host configuration for a benchmark report. |
| `powershell_run` | Run a bounded PowerShell script on Windows. | Invoke an existing maintenance script when no dedicated tool covers it. |
| `file_signature` | Inspect a Windows file's Authenticode status, signer, and version. | Verify the signer of an installed executable. |
| `scheduled_task` | List, inspect, run, stop, enable, disable, register, or unregister Windows tasks. | Check whether a scheduled backup is enabled. |
| `eventlog_follow` | Read Windows Event Log records after an ID and return a cursor. | Poll for new application events during a test. |
| `performance_sample` | Sample CPU, memory, network transfer, and disk capacity. | Record resource usage before and after a workload. |
| `acl_get` | Read a Windows path's owner, SDDL, and access entries. | Diagnose why a process cannot read a file. |
