// Provider-specific browser flow. Credentials never leave this client for Acheflow.
let sdk;
export async function prepare() {
  if (globalThis.google?.accounts?.oauth2) return;
  sdk ??= new Promise((resolve, reject) => {
    const script = document.createElement('script');
    script.src = 'https://accounts.google.com/gsi/client';
    script.onload = resolve;
    script.onerror = () => { sdk = undefined; reject(new Error('SDK unavailable')); };
    document.head.append(script);
  });
  await sdk;
}
// Called directly from the Connect click, preserving the browser's user activation.
export function authorize(clientId, scopes) {
  return new Promise((resolve, reject) => {
    google.accounts.oauth2.initTokenClient({
      client_id: clientId,
      scope: scopes.join(' '),
      include_granted_scopes: false,
      callback: response => response.error ? reject(new Error('Consent rejected')) : resolve(response),
      error_callback: () => reject(new Error('Authorization interrupted')),
    }).requestAccessToken({prompt: 'consent'});
  });
}
