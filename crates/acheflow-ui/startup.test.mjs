import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import test from 'node:test';
const source = () => readFileSync(new URL('./startup.js', import.meta.url), 'utf8');
function fixture({gpu=true, secure=true}={}) {
  const elements=Object.fromEntries(['acheflow-startup','acheflow-startup-title','acheflow-startup-detail','acheflow-startup-retry'].map(id=>[id,{hidden:id.endsWith('retry'),textContent:'',attrs:{},setAttribute(k,v){this.attrs[k]=v;},remove(){this.removed=true;},addEventListener(){}}]));
  const events=new Map(),frames=[],timers=new Map(); let timer=0,complete,reject;
  class Queue {submit(commands){this.commands=commands;}onSubmittedWorkDone(){return new Promise((resolve,fail)=>{complete=resolve;reject=fail;});}}
  class Context {constructor(){this.canvas={nodeName:'CANVAS',isConnected:true,width:100,height:100};}configure(options){this.options=options;}getCurrentTexture(){return 'texture';}}
  const original={configure:Context.prototype.configure,texture:Context.prototype.getCurrentTexture,submit:Queue.prototype.submit};
  const window={addEventListener(name,fn){events.set(name,fn);},removeEventListener(name){events.delete(name);}};
  const document={getElementById:id=>elements[id],documentElement:{dataset:{}}};
  runInNewContext(source(),{window,document,navigator:{gpu:gpu?{}:undefined},isSecureContext:secure,GPUCanvasContext:Context,GPUQueue:Queue,requestAnimationFrame:fn=>frames.push(fn),setTimeout:fn=>{timers.set(++timer,fn);return timer;},clearTimeout:id=>timers.delete(id),location:{reload(){}},console});
  return {elements,events,frames,timers,Context,Queue,original,document,complete:()=>complete(),reject:()=>reject(Error('private failure')),async paint(){await Promise.resolve();while(frames.length)frames.shift()();}};
}
test('canvas creation and offscreen GPU work do not hide the loading screen',async()=>{
 const f=fixture();new f.Queue().submit([]);f.events.get('TrunkApplicationStarted')();await f.paint();
 assert.equal(f.elements['acheflow-startup'].removed,undefined);
 assert.match(f.elements['acheflow-startup-title'].textContent,/Preparing/);
});
test('first canvas submission completes before the loader is removed and hooks are restored',async()=>{
 const f=fixture(),context=new f.Context(),queue=new f.Queue(),options={device:{queue}};
 context.configure(options);assert.equal(context.options,options);assert.equal(context.getCurrentTexture(),'texture');
 const commands=['first frame'];queue.submit(commands);assert.equal(queue.commands,commands);
 assert.equal(f.elements['acheflow-startup'].removed,undefined);f.complete();await f.paint();
 assert.equal(f.elements['acheflow-startup'].removed,true);
 assert.equal(f.Context.prototype.configure,f.original.configure);assert.equal(f.Context.prototype.getCurrentTexture,f.original.texture);assert.equal(f.Queue.prototype.submit,f.original.submit);
 assert.equal(f.events.size,0);assert.equal(f.timers.size,0);
});
test('slow startup remains recoverable without claiming a failed launch',()=>{
 const f=fixture();[...f.timers.values()][0]();
 assert.equal(f.elements['acheflow-startup'].removed,undefined);assert.equal(f.elements['acheflow-startup-retry'].hidden,false);
 assert.match(f.elements['acheflow-startup-detail'].textContent,/longer/);
});
test('unsupported browsers explain HTTPS and WebGPU requirements',()=>{
 for(const options of [{gpu:false},{secure:false}]){const f=fixture(options);assert.match(f.elements['acheflow-startup-title'].textContent,/cannot start/);assert.equal(f.elements['acheflow-startup-retry'].hidden,false);assert.equal(f.Queue.prototype.submit,f.original.submit);}
});
test('GPU failure retains an actionable screen without exposing exception data',async()=>{
 const f=fixture(),queue=new f.Queue(),context=new f.Context();context.configure({device:{queue}});context.getCurrentTexture();queue.submit([]);f.reject();await f.paint();
 assert.equal(f.elements['acheflow-startup'].removed,undefined);assert.match(f.elements['acheflow-startup-title'].textContent,/could not start/);
 assert.equal(f.elements['acheflow-startup-retry'].hidden,false);assert.doesNotMatch(f.elements['acheflow-startup-detail'].textContent,/private failure/);
});
