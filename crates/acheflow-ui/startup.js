// One-shot startup screen. No credentials, network requests or telemetry.
(() => {
  const screen = document.getElementById('acheflow-startup');
  if (!screen) return;
  const title = document.getElementById('acheflow-startup-title');
  const detail = document.getElementById('acheflow-startup-detail');
  const retry = document.getElementById('acheflow-startup-retry');
  const restore = [];
  let finished = false;
  let submitted = false;
  let slowTimer;
  const contexts = new WeakMap();
  const frameQueues = new WeakSet();
  const cleanup = () => {
    clearTimeout(slowTimer);
    for (const undo of restore.reverse()) undo();
    window.removeEventListener('error', onError, true);
    window.removeEventListener('unhandledrejection', onError);
    window.removeEventListener('TrunkApplicationStarted', preparing);
  };
  const fail = (heading, message) => {
    if (finished) return;
    finished = true;
    cleanup();
    screen.setAttribute('aria-busy', 'false');
    screen.setAttribute('data-error', 'true');
    title.textContent = heading;
    detail.textContent = message;
    retry.hidden = false;
  };
  const onError = event => {
    // Missing optional images do not prevent startup. Runtime/module failures do.
    if (event.target && event.target !== window && event.target.tagName !== 'SCRIPT') return;
    fail('Acheflow could not start', 'Reload the page to try again. If this continues, check your connection and browser support for WebGPU.');
  };
  const preparing = () => {
    if (finished) return;
    title.textContent = 'Preparing your studio';
    detail.textContent = 'Initializing the interface and graphics…';
  };
  const ready = () => {
    if (finished) return;
    finished = true;
    cleanup();
    screen.remove();
  };
  const wrap = (prototype, name, replacement) => {
    const original = Object.getOwnPropertyDescriptor(prototype, name);
    if (!original || typeof original.value !== 'function') throw new Error('Unsupported graphics API');
    const wrapped = replacement(original.value);
    Object.defineProperty(prototype, name, {...original, value: wrapped});
    restore.push(() => {
      if (prototype[name] === wrapped) Object.defineProperty(prototype, name, original);
    });
  };
  retry.addEventListener('click', () => location.reload());
  window.addEventListener('error', onError, true);
  window.addEventListener('unhandledrejection', onError);
  window.addEventListener('TrunkApplicationStarted', preparing);
  title.textContent = 'Loading your studio';
  detail.textContent = 'The first visit may take a little longer while Acheflow downloads.';
  if (!isSecureContext || !navigator.gpu) {
    fail('Acheflow cannot start in this browser', 'Open Acheflow over trusted HTTPS in a browser with WebGPU support.');
    return;
  }
  slowTimer = setTimeout(() => {
    if (finished) return;
    detail.textContent = 'This is taking longer than usual. Keep this page open, or check your connection and reload.';
    retry.hidden = false;
  }, 30000);
  try {
    // Canvas insertion precedes asynchronous wgpu initialization. Wait for actual
    // work on its configured queue, not an arbitrary delay or a canvas element.
    wrap(GPUCanvasContext.prototype, 'configure', original => function(options) {
      const result = Reflect.apply(original, this, [options]);
      if (this.canvas?.nodeName === 'CANVAS' && this.canvas.isConnected) {
        contexts.set(this, options.device.queue);
        preparing();
      }
      return result;
    });
    wrap(GPUCanvasContext.prototype, 'getCurrentTexture', original => function(...args) {
      const texture = Reflect.apply(original, this, args);
      const queue = contexts.get(this);
      if (queue) frameQueues.add(queue);
      return texture;
    });
    wrap(GPUQueue.prototype, 'submit', original => function(...args) {
      const result = Reflect.apply(original, this, args);
      if (!submitted && frameQueues.has(this)) {
        submitted = true;
        this.onSubmittedWorkDone().then(
          () => requestAnimationFrame(() => requestAnimationFrame(ready)),
          () => fail('Acheflow could not start', 'Graphics initialization stopped. Reload the page to try again.')
        );
      }
      return result;
    });
  } catch {
    fail('Acheflow could not start', 'This browser could not initialize the graphics interface. Update the browser and reload.');
  }
})();
