import { attach } from './page.mjs';
import { attachRelay } from './relay.mjs';
import { callbackTarget } from './callback.mjs';
let address = location.href;
if (document.documentElement.dataset.recipient === 'browser') {
  history.replaceState(null, '', '/');
  try {
    const target = callbackTarget(address, location.origin);
    if (target.target === 'native') {
      document.getElementById('description').textContent = 'Complete sign-in in the installed Acheflow application on this computer.';
      attach(document, target.address, history, url => location.assign(url), 'client');
    } else {
      attachRelay(document, window, address, target.fields);
    }
  } catch {
    document.getElementById('status').textContent = 'This authorization could not be matched to a client. Return to the original client and start sign-in again.';
  }
} else {
  attach(document, address, history, url => location.assign(url));
}
address = '';
