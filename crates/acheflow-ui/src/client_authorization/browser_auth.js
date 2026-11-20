export async function prepare(source) {
  const url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
  try { const module = await import(url); await module.prepare(); return module; }
  finally { URL.revokeObjectURL(url); }
}

export function loadRoute() {
  try { return window.localStorage.getItem('acheflow:browser-authorization-route') !== 'direct'; }
  catch { return true; }
}
export function saveRoute(hosted) {
  try { window.localStorage.setItem('acheflow:browser-authorization-route', hosted ? 'hosted' : 'direct'); }
  catch { /* Route selection still works when browser preferences cannot persist. */ }
}

export function start(url, callback, module, clientId, scopes, proxy = 'https://app.acheflow.com/callback/', host = window) {
  const expected = new URL(callback);
  const hosted = expected.href === new URL(proxy).href;
  if (hosted && (expected.protocol !== 'https:' || expected.username || expected.password || expected.search || expected.hash)) throw new Error('Invalid callback proxy');
  if (!module && !hosted && (expected.origin !== host.location.origin || expected.protocol !== 'https:' || expected.search || expected.hash)) {
    throw new Error('Invalid callback destination');
  }
  let popup, timer, rejectAttempt, finished = false, ready = false, receive;
  const cleanup = () => {
    clearInterval(timer);
    if (receive) host.removeEventListener('message', receive);
  };
  const close = () => { try { if (popup && !popup.closed) popup.close(); } catch {} };
  const promise = new Promise((resolve, reject) => {
    rejectAttempt = reject;
    const fail = () => { if (!finished) { finished = true; cleanup(); close(); reject(new Error('Authorization interrupted')); } };
    const accept = value => { if (!finished) { finished = true; cleanup(); close(); resolve(value); } };
    if (module) {
      // Called synchronously from the user's click so the provider can open its UI.
      module.authorize(clientId, JSON.parse(scopes)).then(value => accept(JSON.stringify(value)), fail);
      timer = setTimeout(fail, 300000);
      return;
    }
    let challenge;
    if (hosted) {
      const bytes = host.crypto.getRandomValues(new Uint8Array(32));
      challenge = btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replaceAll('=','');
      receive = event => {
        if (finished || event.source !== popup || event.origin !== expected.origin || event.data?.challenge !== challenge) return;
        if (event.data.type === 'acheflow:callback-ready') ready = true;
        if (event.data.type === 'acheflow:callback-result' && ready && typeof event.data.fields === 'string' && event.data.fields.length <= 16384) accept(event.data.fields);
      };
      host.addEventListener('message', receive);
    }
    popup = host.open(url, '_blank', 'popup,width=600,height=760');
    if (!popup) { fail(); return; }
    if (!hosted) { try { popup.opener = null; } catch {} }
    const deadline = Date.now() + 300000;
    const poll = () => {
      if (finished) return;
      if (Date.now() > deadline || popup.closed) { fail(); return; }
      if (hosted) {
        // No credentials are sent in probes; the exact origin is always pinned.
        try { popup.postMessage({type:'acheflow:callback-probe', challenge}, expected.origin); } catch {}
        return;
      }
      try {
        const current = new URL(popup.location.href);
        if (current.origin !== expected.origin || current.pathname !== expected.pathname) return;
        const fields = current.hash.slice(1) || current.search.slice(1);
        popup.history.replaceState(null, '', expected.pathname);
        accept(fields);
      } catch { /* Provider pages are intentionally cross-origin. */ }
    };
    timer = setInterval(poll, 250);
    poll();
  });
  return {promise, cancel() {
    if (finished) return;
    finished = true; cleanup(); close(); rejectAttempt(new Error('Authorization cancelled'));
  }};
}
export function wait(handle) { return handle.promise; }
export function cancel(handle) { handle.cancel(); }
