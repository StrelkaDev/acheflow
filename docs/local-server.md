# Desktop-owned local server

DL-09 supplies the native `acheflow_client::server` lifecycle service. Desktop
Settings controls and saved startup/exit preferences are the next task, DL-10;
the service does not start a server automatically. Browser builds do not include
this module. Unified executable/URI dispatch remains DL-13.

## Launch and ownership

`LaunchConfig::bundled(directory)` resolves a fixed `acheflow-server` sibling of
the client executable (`.exe` on Windows), including a future macOS
`Contents/MacOS` layout. There is no PATH search or shell. Until DL-11 bundles
both executables, a development build can place them together in `target/debug`
or pass an explicit absolute binary path. Missing binaries fail with an actionable
status. Data/configuration uses an explicit absolute application-data directory,
never the Finder/caller working directory.

`LocalServer::new()` runs inside a Tokio runtime. `start(config)` first checks
`acheflow-server --version`, with a three-second deadline and bounded output;
the version must exactly match the client's package version. It then checks the
requested HTTP and RTMP ports, spawns the child with argument-array invocation,
and waits for readiness. HTTP defaults to `127.0.0.1:21295`, RTMP to
`127.0.0.1:1935`. Both ports are configurable and distinct. The process loads
`acheflow-server.json` from its data directory, while launch arguments override
both listeners to loopback. Managed mode rejects TLS settings; standalone/remote
servers retain their existing HTTP(S)/RTMP(S) configuration.

Standalone listener/TLS/config and MCP credential environment overrides are
removed from the child's environment. FFmpeg discovery/PATH and existing
extension installation discovery remain available. The lifecycle manager neither
stores nor sends provider credentials.

## Readiness and state

`GET /api/server/info` returns `ServerInfo`: `protocol_version` (currently 1),
package `version`, `process_id`, optional `managed_instance`, and actual HTTP and
RTMP socket addresses. Ordinary servers return `managed_instance: null`.
The managed launch supplies a fresh 128-bit correlation ID through
`ACHEFLOW_DESKTOP_INSTANCE`; this is public identity metadata, not an access or
shutdown credential. A successful `/healthz` or another process on the port
cannot establish ownership.

Readiness requires the expected protocol/version, exact child PID, launch ID,
and configured addresses. Startup defaults to 30 seconds and is configurable
from 100 milliseconds to 60 seconds after spawning. Local probes bypass proxies,
follow no redirects, have a 400-millisecond total request timeout, and accept at
most 4 KiB of JSON. The manager polls process/API state every 300 milliseconds;
it reports `Starting`, `Ready`, `Unavailable`, `Stopping`, `Stopped`, `Exited`
or `Failed`. An owned process with a temporarily unavailable API stays owned and
can be stopped. `inspect_local(port)` reads identity without ever adopting a
process. A conflict or identity mismatch never signals the external process.

`status()` returns a snapshot; `subscribe()` returns a watch receiver. Exit/error
state and the last identity remain available for diagnostics, while `pid` is
cleared once the owned child is reaped. `diagnostics()` retains at most 64 KiB and
128 records of stdout/stderr, continuously draining fixed-size reads even when
the child emits unbroken lines or invalid UTF-8. Control characters are removed.
Start clears the previous run's output; stop keeps the final output.

## Stop, restart and cancellation

The server's explicit `--desktop-managed` mode observes its private stdin pipe;
EOF, a byte, or a read error initiates the existing graceful shutdown path. This
mode conflicts with native activation flags and cannot hand off to another
process. Ordinary standalone server stdin behavior is unchanged. Native server
link reception remains the separate opt-in launcher path.

`stop()` closes only the captured child pipe, waits up to twelve seconds, and
force-kills/reaps that same `Child` if needed (a further two-second wait).
`forced_stop` distinguishes fallback termination. `restart(config)` awaits stop
before starting a fresh instance. Dropping the sole lifecycle handle asks its
worker to stop the child; closing/crashing the parent also closes the pipe.
Applications should await `stop()` before shutting down their Tokio runtime;
the final `kill_on_drop` guard prevents abandonment if its worker is aborted.
Tokio's child ownership/cancellation behavior is documented in its
[process API](https://docs.rs/tokio/latest/tokio/process/struct.Child.html).

Canceling the caller's `start()` future does not discard ownership: its worker
continues the bounded startup and accepts a subsequent stop. The API owns no PID
obtained from an HTTP response or a persisted file. A second manager may inspect
the first manager's server, but cannot stop it.

## Verification

`cargo test --offline --locked -p acheflow-server --test owned_server` exercises
the production executable for readiness, matching identity/version, saved-listener
overrides, graceful stop/restart, owner drop, crash and cross-manager isolation.
Small real executable fixtures exercise incompatible versions, startup timeout,
large diagnostic output, canceled startup, wrong launch identity and forced
termination. Tests use temporary directories and ephemeral loopback ports;
socket-restricting sandboxes need explicit local-test permission. The bounded
diagnostic reader is also checked in the client library without sockets.
