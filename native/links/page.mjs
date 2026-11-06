// URLs stay in this closure; no storage, telemetry, fetch or automatic activation.
export function applicationLink(input, recipient) {
  if (!['client', 'server'].includes(recipient)) throw new Error('Invalid recipient');
  if (new TextEncoder().encode(input).length > 16 * 1024 || /[\x00-\x20\x7f\\]/.test(input)) throw new Error('Invalid URL');
  const origin = recipient === 'client' ? 'https://app.acheflow.com' : 'https://server.acheflow.com';
  if (!input.startsWith(`${origin}/`)) throw new Error('Wrong origin');
  const url = new URL(input);
  const relative = input.slice(origin.length);
  const path = relative.split(/[?#]/, 1)[0];
  if (path !== url.pathname) throw new Error('Invalid path');
  validateRoute(path);
  validateEscapes(path, true);
  validateEscapes(url.search, false);
  validateEscapes(url.hash, false);
  return `acheflow-${recipient}:/${relative}`;
}

function validateRoute(path) {
  if (!path.startsWith('/fwd/')) throw new Error('Invalid path');
  const [id, action, ...tail] = path.slice(5).split('/');
  const validID = value => /^[A-Za-z0-9_-]{1,64}$/.test(value ?? '');
  const remainder = tail.join('/');
  if (!validID(id) || ['acheflow', 'ui'].includes(id) || !validID(action) || remainder.startsWith('/') || remainder.includes('//')) throw new Error('Invalid route');
}

function validateEscapes(value, isPath) {
  for (let i = 0; i < value.length; i++) {
    if (value[i] !== '%') continue;
    const hex = value.slice(i + 1, i + 3);
    if (!/^[\da-f]{2}$/i.test(hex)) throw new Error('Invalid escape');
    const byte = parseInt(hex, 16);
    if (byte < 32 || byte === 127 || (isPath && [37, 47, 92].includes(byte))) throw new Error('Invalid escape');
    i += 2;
  }
}

export function attach(page, address, history, navigate) {
  // Remove path/query/fragment from history before any user leaves this page.
  history.replaceState(null, '', '/');
  const button = page.getElementById('open');
  const status = page.getElementById('status');
  try {
    const target = applicationLink(address, page.documentElement.dataset.recipient);
    button.disabled = false;
    status.textContent = 'Ready. Open the installed application to continue.';
    button.addEventListener('click', () => {
      status.textContent = 'If Acheflow did not open, install it and return to the original link.';
      try { navigate(target); } catch { status.textContent = 'Your browser could not open Acheflow. Install it and try the original link again.'; }
    });
  } catch {
    button.disabled = true;
    status.setAttribute('role', 'alert');
    status.textContent = 'This link is invalid for this application. Return to the original application and request a new link.';
  }
}

