import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applicationLink, attach } from './page.mjs';

test('exact recipient and opaque callback encoding survive the fallback', () => {
  for (const recipient of ['client', 'server']) {
    const path = '/fwd/example/oauth/callback?code=a%2Bb&state=x%3Dy#other=opaque';
    assert.equal(applicationLink(`https://${recipient}.acheflow.com${path}`, recipient), `acheflow-${recipient}:/${path}`);
    assert.equal(applicationLink(`https://${recipient}.acheflow.com/fwd/example/open/`, recipient), `acheflow-${recipient}://fwd/example/open/`);
  }
});

test('wrong hosts, route aliases and malformed escapes never produce an OS URL', () => {
  for (const url of [
    'https://server.acheflow.com/fwd/example/open',
    'https://client.acheflow.com.evil/fwd/example/open',
    'https://client.acheflow.com:443/fwd/example/open',
    'https://client.acheflow.com@evil/fwd/example/open',
    'https://client.acheflow.com/fwd/ui/open',
    'https://client.acheflow.com/fwd/example/../open',
    'https://client.acheflow.com/fwd/example/open/%2Fsecret',
    'https://client.acheflow.com/fwd/example/open//tail',
    'https://client.acheflow.com/fwd/example/open?bad=%',
    'https://client.acheflow.com/fwd/example/open?bad=%0a',
    'https://client.acheflow.com/fwd/example/open\n',
    `https://client.acheflow.com/fwd/example/open?bad=${'a'.repeat(16384)}`,
  ]) assert.throws(() => applicationLink(url, 'client'));
});

test('no automatic launch, no visible payload and history is scrubbed first', () => {
  const calls = [];
  const elements = { open: { disabled: true, addEventListener: (_, callback) => { elements.open.click = callback; } }, status: { setAttribute: () => {} } };
  const page = { documentElement: { dataset: { recipient: 'client' } }, getElementById: id => elements[id] };
  const history = { replaceState: (...args) => calls.push(args) };
  const source = 'https://client.acheflow.com/fwd/example/open?code=sensitive-marker';
  attach(page, source, history, url => calls.push(url));
  assert.deepEqual(calls, [[null, '', '/']]);
  assert.equal(elements.open.disabled, false);
  assert.ok(!elements.status.textContent.includes('sensitive-marker'));
  elements.open.click();
  assert.equal(calls[1], 'acheflow-client://fwd/example/open?code=sensitive-marker');
  attach(page, 'https://evil.test/fwd/example/open', history, () => assert.fail('invalid launch'));
  assert.equal(elements.open.disabled, true);
  assert.ok(!elements.status.textContent.includes('evil.test'));
});
