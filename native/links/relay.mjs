// Only in-memory browser transport. No token storage, fetch, or URL forwarding.
export function attachRelay(page, host, address, normalizedFields) {
  const button = page.getElementById('open');
  const status = page.getElementById('status');
  button.disabled = true;
  button.textContent = 'Return to browser studio';
  status.textContent = 'Waiting for the browser studio. If it was closed or your browser disconnected the windows, return to the studio and start sign-in again.';
  if (!host.opener) {
    button.textContent = 'Return to Acheflow';
    status.textContent = 'No waiting client could be found. Return to the Acheflow client that started sign-in and try again. Check the registered callback address if authorization was rejected.';
    return;
  }
  const url = new URL(address);
  let fields = normalizedFields ?? (url.hash.slice(1) || url.search.slice(1));
  normalizedFields = undefined;
  url.hash = '';
  url.search = '';
  address = '';
  if (!fields || fields.length > 16384) return;
  const opener = host.opener;
  let destination, challenge, finished = false;
  const expire = () => {
    finished = true;
    fields = '';
    host.removeEventListener('message', receive);
    host.clearTimeout(timer);
    button.disabled = true;
  };
  const receive = event => {
    if (finished || event.source !== opener || event.data?.type !== 'acheflow:callback-probe') return;
    let origin;
    try { origin = new URL(event.origin); } catch { return; }
    if (origin.protocol !== 'https:' || origin.origin !== event.origin) return;
    if (!/^[A-Za-z0-9_-]{43}$/.test(event.data.challenge ?? '')) return;
    if (destination) return;
    destination = event.origin;
    challenge = event.data.challenge;
    button.disabled = false;
    button.textContent = 'Return to browser studio';
    page.getElementById('description').textContent = 'Finish sign-in in the browser studio that opened this window.';
    status.textContent = `Return authorization to ${destination}. The callback is held only in this page’s memory; it is not saved by this site.`;
    opener.postMessage({type:'acheflow:callback-ready', challenge}, destination);
  };
  host.addEventListener('message', receive);
  const timer = host.setTimeout(() => {
    expire();
    status.textContent = 'This sign-in attempt expired. Return to your studio and start again.';
  }, 300000);
  host.addEventListener('pagehide', expire, {once:true});
  button.addEventListener('click', () => {
    if (finished || !destination) return;
    opener.postMessage({type:'acheflow:callback-result', challenge, fields}, destination);
    expire();
    status.textContent = 'Authorization returned. You can close this window.';
    host.close();
  });
  return expire;
}
