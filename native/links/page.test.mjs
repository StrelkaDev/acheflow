import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applicationLink, attach } from './page.mjs';

test('shared app domain preserves the extension callback and never routes it to a server', () => {
  const path='/fwd/twitch/oauth/callback?state=nonce#access_token=opaque';
  assert.equal(applicationLink(`https://app.acheflow.com${path}`,'client'),`acheflow://client${path}`);
  assert.throws(()=>applicationLink(`https://app.acheflow.com${path}`,'server'));
  assert.throws(()=>applicationLink(`https://app.acheflow.com.evil${path}`,'client'));
});

test('exact recipient and opaque callback encoding survive the fallback', () => {
  for (const recipient of ['client', 'server']) {
    const origin = recipient === 'client' ? 'https://app.acheflow.com' : 'https://app.acheflow.com/server';
    const path = '/fwd/example/oauth/callback?code=a%2Bb&state=x%3Dy#other=opaque';
    assert.equal(applicationLink(`${origin}${path}`, recipient), `acheflow://${recipient}${path}`);
    assert.equal(applicationLink(`${origin}/fwd/example/open/`, recipient), `acheflow://${recipient}/fwd/example/open/`);
  }
});

test('wrong hosts, route aliases and malformed escapes never produce an OS URL', () => {
  for (const url of [
    'https://client.acheflow.com/fwd/example/open',
    'https://server.acheflow.com/fwd/example/open',
    'https://app.acheflow.com.evil/fwd/example/open',
    'https://app.acheflow.com:443/fwd/example/open',
    'https://app.acheflow.com@evil/fwd/example/open',
    'https://app.acheflow.com/fwd/ui/open',
    'https://app.acheflow.com/fwd/example/../open',
    'https://app.acheflow.com/fwd/example/open/%2Fsecret',
    'https://app.acheflow.com/fwd/example/open//tail',
    'https://app.acheflow.com/fwd/example/open?bad=%',
    'https://app.acheflow.com/fwd/example/open?bad=%0a',
    'https://app.acheflow.com/fwd/example/open\n',
    `https://app.acheflow.com/fwd/example/open?bad=${'a'.repeat(16384)}`,
  ]) assert.throws(() => applicationLink(url, 'client'));
});

test('no automatic launch, no visible payload and history is scrubbed first', () => {
  const calls = [];
  const elements = { open: { disabled: true, addEventListener: (_, callback) => { elements.open.click = callback; } }, status: { setAttribute: () => {} } };
  const page = { documentElement: { dataset: { recipient: 'client' } }, getElementById: id => elements[id] };
  const history = { replaceState: (...args) => calls.push(args) };
  const source = 'https://app.acheflow.com/fwd/example/open?code=sensitive-marker';
  attach(page, source, history, url => calls.push(url));
  assert.deepEqual(calls, [[null, '', '/']]);
  assert.equal(elements.open.disabled, false);
  assert.ok(!elements.status.textContent.includes('sensitive-marker'));
  elements.open.click();
  assert.equal(calls[1], 'acheflow://client/fwd/example/open?code=sensitive-marker');
  attach(page, 'https://evil.test/fwd/example/open', history, () => assert.fail('invalid launch'));
  assert.equal(elements.open.disabled, true);
  assert.ok(!elements.status.textContent.includes('evil.test'));
});

test('server route cannot replace a client callback or claim ordinary browser navigation', () => {
  assert.equal(applicationLink('https://app.acheflow.com/server/fwd/example/open?value=a%2Bb#x=y', 'server'), 'acheflow://server/fwd/example/open?value=a%2Bb#x=y');
  assert.throws(() => applicationLink('https://app.acheflow.com/server/fwd/example/open', 'client'));
  assert.throws(() => applicationLink('https://app.acheflow.com/', 'client'));
  assert.throws(() => applicationLink('https://app.acheflow.com/server/fwd/example/../open', 'server'));
});
