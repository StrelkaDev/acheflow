import {test} from 'node:test';
import assert from 'node:assert/strict';
import {attachRelay} from './relay.mjs';
function fixture(){
  const sent=[],handlers=new Map();const opener={postMessage:(...args)=>sent.push(args)};
  const elements={open:{disabled:true,addEventListener:(name,fn)=>handlers.set(name,fn)},status:{setAttribute(){}},description:{}};
  const page={getElementById:id=>elements[id]};
  let receive;const host={opener,addEventListener:(name,fn)=>{if(name==='message')receive=fn;},removeEventListener:()=>{},setTimeout:fn=>{host.expire=fn;return 1;},clearTimeout:()=>{},close:()=>{host.closed=true;}};
  attachRelay(page,host,'https://app.acheflow.com/fwd/example/oauth/callback#access_token=opaque&state=nonce');
  const send=(source=opener,origin='https://studio.example',data={type:'acheflow:callback-probe',challenge:'c'.repeat(43)})=>receive({source,origin,data});
  return {sent,host,elements,handlers,send,opener};
}
test('relay releases callback only to the original HTTPS opener after explicit confirmation',()=>{
 const f=fixture();f.send();assert.equal(f.sent.length,1);assert.equal(f.sent[0][0].type,'acheflow:callback-ready');assert.ok(!JSON.stringify(f.sent).includes('opaque'));
 assert.match(f.elements.status.textContent,/https:\/\/studio.example/);assert.equal(f.elements.open.disabled,false);
 f.handlers.get('click')();assert.equal(f.sent[1][1],'https://studio.example');assert.equal(f.sent[1][0].fields,'access_token=opaque&state=nonce');assert.equal(f.host.closed,true);
 f.handlers.get('click')();assert.equal(f.sent.length,2);
});
test('wrong source, insecure origin and malformed challenges never receive credentials',()=>{
 for(const action of [f=>f.send({}),f=>f.send(f.opener,'http://evil.example'),f=>f.send(f.opener,'null'),f=>f.send(f.opener,'https://studio.example',{type:'acheflow:callback-probe',challenge:'short'})]){
  const f=fixture();action(f);assert.equal(f.sent.length,0);assert.equal(f.elements.open.disabled,true);
 }
});
test('a bound destination cannot be replaced by another origin',()=>{const f=fixture();f.send();f.send(f.opener,'https://other.example');f.handlers.get('click')();assert.equal(f.sent.at(-1)[1],'https://studio.example');});
test('expiration clears access to the callback without delivering it',()=>{const f=fixture();f.send();f.host.expire();f.handlers.get('click')();assert.equal(f.sent.length,1);assert.equal(f.elements.open.disabled,true);assert.match(f.elements.status.textContent,/expired/);});
