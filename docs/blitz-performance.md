# Browser sparks performance — 2026-10-31

These are historical measurements of the Overview sparks scene on 2026-10-31.
Artifact sizes and frame cadence below apply to that recorded build and scene;
live composition and audio-mixer performance were not measured here.

## Finding and fix

The served browser prototype was debug WASM. Its unoptimized Rust UI/rendering
engine could not keep up with continuous redraw. Switching the same scene and
180-spark WGSL shader to release WASM restored the browser's observed rAF cadence
from about 6/s to 60/s. `Trunk.toml` now uses `release = true` by default.
Browser tracing now enables WARN/ERROR only and disables Performance Timeline
reporting; the old default TRACE subscriber accumulated timing entries over time.

Blitz's current `View::redraw` resolves the document and calls `paint_scene` for
the complete scene whenever a custom widget requests another frame. The effect
therefore triggers UI scene regeneration/compositing as well as the particle
pass. This source-level behavior is confirmed; the individual CPU/GPU phase costs
were not separately profiled. No engine caching, shader reduction or reduced
render resolution was needed for this fix.

## Measurements

Same in-app browser tab, Overview route, WebGPU adapter, 1798 × 1908 canvas
backing pixels, same CSS and 180 particles. `perf.js` measures main-thread
requestAnimationFrame intervals; these are frame-pacing observations, not GPU
timestamp queries or proof that every callback corresponds to a presented frame.
The final comparison was taken after background compilation finished, with the
page visible. No viewport/DPI emulation was active.

| Variant | Samples | Mean interval | p50 | p95 | rAF/s | Intervals >50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Original debug + TRACE (first observation, includes startup) | 106 | 164.62 ms | 166.7 ms | 183.4 ms | 6.1 | 100 |
| Original debug + TRACE (later observation) | 556 | 171.31 ms | 166.7 ms | 183.4 ms | 5.8 | 550 |
| Debug, TRACE disabled (clean window after build) | 120 | 173.06 ms | 166.7 ms | 184.3 ms | 5.8 | 120 |
| Release, TRACE disabled (clean window after build) | 120 | 16.67 ms | 16.7 ms | 17.7 ms | 60.0 | 0 |

Disabling TRACE alone did not materially improve cadence. It is retained to
prevent unbounded diagnostic timeline accumulation: the original run grew from
781 to 3637 measure entries, while the changed run remained at zero.
The optimized WASM artifact is about 9.6 MiB versus about 117 MiB in debug.
This is uncompressed size, not a network throughput measurement.

## Reproduce / guard

```sh
trunk build
cargo run -p acheflow-server --release --locked
```

Open `http://localhost:8080/?perf=1`, wait for the first frame plus five seconds
of warm-up and at least 120 samples, then read
`document.documentElement.dataset.acheflowPerf` from developer tools.
At this scene/size on the current machine, a repeatable p95 above 33.4 ms is an
investigation trigger. This is a manual local regression check, not a CI gate or
portable hardware performance guarantee. Compare visible tabs on the same
machine without a concurrent build. Run normal URLs without `?perf=1` outside
measurement sessions.
