import {applicationLink} from './page.mjs';

// Transport metadata only. The waiting client owns workspace/instance identity,
// permissions and expiry; this page never chooses or authorizes a workspace.
export function callbackTarget(address, expectedOrigin = 'https://app.acheflow.com') {
  const callback = new URL(address);
  if (callback.origin !== expectedOrigin || callback.protocol !== 'https:') throw new Error('Invalid callback origin');
  if (callback.pathname !== '/callback' && !callback.pathname.startsWith('/callback/')) throw new Error('Invalid callback path');
  const raw = callback.hash.slice(1) || callback.search.slice(1);
  if (raw.length > 16384) throw new Error('Callback too large');
  const fields = new URLSearchParams(raw);
  const tagged = [...fields].filter(([,value]) => value.startsWith('acheflow-callback:'));
  if (!tagged.length) return {target:'browser', fields:raw};
  if (tagged.length !== 1) throw new Error('Ambiguous callback');
  const [field, state] = tagged[0];
  if (fields.getAll(field).length !== 1 || state.length > 2048) throw new Error('Invalid callback');
  const envelope = JSON.parse(state.slice('acheflow-callback:'.length));
  const allowed = ['v','target','nonce','route'];
  if (envelope.v !== 1 || Object.keys(envelope).some(key => !allowed.includes(key)) || !/^[A-Za-z0-9_-]{43}$/.test(envelope.nonce ?? '')) throw new Error('Invalid callback');
  if (envelope.target === 'browser' && envelope.route === undefined) {
    fields.set(field, envelope.nonce);
    return {target:'browser', fields:fields.toString()};
  }
  if (envelope.target !== 'native' || typeof envelope.route !== 'string' || !envelope.route.startsWith('/fwd/') || /[?#]/.test(envelope.route)) throw new Error('Invalid callback');
  const base = 'https://app.acheflow.com' + envelope.route;
  // Validate spelling/escapes before URLSearchParams normalizes the carrier.
  applicationLink(base + (callback.hash ? '#' : '?') + raw, 'client');
  fields.set(field, envelope.nonce);
  return {target:'native', address:base + (callback.hash ? '#' : '?') + fields.toString()};
}
export function nativeCallback(address) {
  const target = callbackTarget(address);
  return target.target === 'native' ? target.address : null;
}
