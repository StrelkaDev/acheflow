// Opt-in, local frame-pacing diagnostics: /?perf=1. No telemetry is sent.
if (new URLSearchParams(location.search).get('perf') === '1') {
  const intervals = [];
  let previous = 0;
  let lastReport = 0;
  let readySince = 0;
  const tick = (now) => {
    if (!document.querySelector('canvas')) {
      previous = 0;
      requestAnimationFrame(tick);
      return;
    }
    if (!readySince) readySince = now;
    if (now - readySince < 5000) {
      previous = 0;
      requestAnimationFrame(tick);
      return;
    }
    if (previous && document.visibilityState === 'visible') {
      intervals.push(now - previous);
      if (intervals.length > 120) intervals.shift();
    }
    previous = now;
    if (now - lastReport > 1000 && intervals.length) {
      const sorted = [...intervals].sort((a,b) => a-b);
      const mean = intervals.reduce((a,b) => a+b, 0) / intervals.length;
      document.documentElement.dataset.acheflowPerf = JSON.stringify({
        samples: intervals.length,
        visibility: document.visibilityState,
        meanMs: +mean.toFixed(2),
        p50Ms: +sorted[Math.floor(sorted.length * 0.5)].toFixed(2),
        p95Ms: +sorted[Math.floor(sorted.length * 0.95)].toFixed(2),
        maxMs: +sorted.at(-1).toFixed(2),
        rafPerSecond: +(1000/mean).toFixed(1),
        over50ms: intervals.filter(ms => ms > 50).length,
        tracingMeasures: performance.getEntriesByType('measure').length,
      });
      lastReport = now;
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}
