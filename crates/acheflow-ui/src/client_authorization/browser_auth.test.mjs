import {test} from 'node:test';
import assert from 'node:assert/strict';
import {start} from './browser_auth.js';
function fixture(callback='https://app.acheflow.com/callback/',proxy='https://app.acheflow.com/callback/'){
 const listeners=new Set(),sent=[];
 const popup={closed:false,opener:{},location:{get href(){throw Error('Cross origin');}},postMessage:(...args)=>sent.push(args),close(){this.closed=true;}};
 const host={crypto:globalThis.crypto,location:{origin:'https://studio.example'},open:()=>popup,addEventListener:(_,fn)=>listeners.add(fn),removeEventListener:(_,fn)=>listeners.delete(fn)};
 const handle=start('https://provider.example/authorize',callback,null,'','[]',proxy,host);
 const message=(data,source=popup,origin='https://app.acheflow.com')=>{for(const fn of listeners)fn({source,origin,data});};
 return {handle,popup,host,sent,listeners,message};
}
test('hosted callback requires matching popup, origin, challenge and readiness before accepting',async()=>{
 const f=fixture();try {
  const challenge=f.sent[0][0].challenge;
  f.message({type:'acheflow:callback-result',challenge,fields:'forged'});
  f.message({type:'acheflow:callback-ready',challenge},{},'https://app.acheflow.com');
  f.message({type:'acheflow:callback-ready',challenge},f.popup,'https://evil.example');
  f.message({type:'acheflow:callback-ready',challenge});
  f.message({type:'acheflow:callback-result',challenge:'x'.repeat(43),fields:'forged'});
  f.message({type:'acheflow:callback-result',challenge,fields:'state=expected&access_token=opaque'});
  assert.equal(await f.handle.promise,'state=expected&access_token=opaque');assert.equal(f.listeners.size,0);assert.equal(f.popup.closed,true);
 }finally{f.handle.cancel();}
});
test('cancel removes message listener, closes popup, and rejects pending authorization',async()=>{
 const f=fixture();f.handle.cancel();await assert.rejects(f.handle.promise);assert.equal(f.listeners.size,0);assert.equal(f.popup.closed,true);
});
test('direct callback detaches opener and never accepts cross-origin postMessage',async()=>{
 const f=fixture('https://studio.example/oauth-callback.html');assert.equal(f.popup.opener,null);assert.equal(f.listeners.size,0);f.handle.cancel();await assert.rejects(f.handle.promise);
});
test('an explicitly configured private callback host is pinned for browser delivery',async()=>{
 const uri='https://callbacks.example/callback/';const f=fixture(uri,uri);
 try{const challenge=f.sent[0][0].challenge;
  f.message({type:'acheflow:callback-ready',challenge},f.popup,'https://callbacks.example');
  f.message({type:'acheflow:callback-result',challenge,fields:'state=expected'},f.popup,'https://app.acheflow.com');
  f.message({type:'acheflow:callback-result',challenge,fields:'state=expected'},f.popup,'https://callbacks.example');
  assert.equal(await f.handle.promise,'state=expected');assert.equal(f.sent[0][1],'https://callbacks.example');
 }finally{f.handle.cancel();}
});
test('unapproved callback destinations are rejected before opening a window',()=>{
 for(const callback of ['https://evil.example/callback','https://app.acheflow.com/fwd/example/open','http://studio.example/callback'])assert.throws(()=>fixture(callback));
});
test('direct callback reads the matching page once and removes its credentials from history',async t=>{
 t.mock.timers.enable({apis:['setInterval']});
 const f=fixture('https://studio.example/oauth-callback.html');
 let cleaned;
 f.popup.location={href:'https://studio.example/oauth-callback.html#state=expected&access_token=opaque'};
 f.popup.history={replaceState:(_,__,path)=>{cleaned=path;}};
 t.mock.timers.tick(250);
 assert.equal(await f.handle.promise,'state=expected&access_token=opaque');assert.equal(cleaned,'/oauth-callback.html');assert.equal(f.popup.closed,true);
});
