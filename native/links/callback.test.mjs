import {test} from 'node:test';
import assert from 'node:assert/strict';
import {nativeCallback,callbackTarget} from './callback.mjs';
const nonce='n'.repeat(43);
function callback(route='/fwd/example/oauth/callback',extra={}){
 const fields=new URLSearchParams({state:'acheflow-callback:'+JSON.stringify({v:1,target:'native',route,nonce,...extra}),access_token:'opaque',token_type:'bearer'});
 return 'https://app.acheflow.com/callback/#'+fields;
}
test('one shared callback restores only the declared native route and original nonce',()=>{
 const target=new URL(nativeCallback(callback()));assert.equal(target.origin,'https://app.acheflow.com');assert.equal(target.pathname,'/fwd/example/oauth/callback');
 const fields=new URLSearchParams(target.hash.slice(1));assert.equal(fields.get('state'),nonce);assert.equal(fields.get('access_token'),'opaque');
});
test('ordinary browser state never becomes an application activation',()=>{assert.equal(nativeCallback('https://app.acheflow.com/callback/#state='+nonce),null);});
test('the same static callback router works on a separately configured HTTPS host',()=>{
 const custom=callback().replace('https://app.acheflow.com','https://callbacks.example');
 assert.equal(callbackTarget(custom,'https://callbacks.example').target,'native');
 assert.throws(()=>callbackTarget(custom,'https://different.example'));
});
test('untrusted routes and malformed native envelopes cannot choose another destination',()=>{
 for(const route of ['https://evil.example/fwd/example/oauth/callback','//evil.example/','/server/fwd/example/oauth/callback','/fwd/ui/oauth/callback','/fwd/example/oauth/callback?x=y','/fwd/example/oauth/../callback'])assert.throws(()=>nativeCallback(callback(route)));
 assert.throws(()=>nativeCallback(callback(undefined,{nonce:'short'})));
 assert.throws(()=>nativeCallback(callback()+'&state=other'));
});

test('a non-OAuth callback field carries the same browser request envelope',()=>{
 const fields=new URLSearchParams({receipt:'acheflow-callback:'+JSON.stringify({v:1,target:'browser',nonce}),result:'fixture'});
 const target=callbackTarget('https://app.acheflow.com/callback#'+fields);
 assert.equal(target.target,'browser');assert.equal(new URLSearchParams(target.fields).get('receipt'),nonce);
});
test('unknown recipient, version and competing envelopes are rejected',()=>{
 for(const extra of [{target:'server'},{v:2},{workspace:'untrusted'}])assert.throws(()=>callbackTarget(callback(undefined,extra)));
 assert.throws(()=>callbackTarget(callback()+'&another='+encodeURIComponent('acheflow-callback:'+JSON.stringify({v:1,target:'browser',nonce}))));
});

test('contextual callback paths preserve correlation and cannot select the native route',()=>{
 const target=callbackTarget(callback().replace('/callback/#','/callback/account/connect#'));
 assert.equal(target.target,'native');
 assert.equal(new URL(target.address).pathname,'/fwd/example/oauth/callback');
});
test('the removed public OAuth path cannot deliver a callback from a cached page',()=>{
 assert.throws(()=>callbackTarget(callback().replace('/callback/#','/oauth/callback#')));
});
