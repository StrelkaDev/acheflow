import test from 'node:test';
import assert from 'node:assert/strict';
import {installMobileInput} from './mobile_input.js';
class Element {
  constructor(tag){this.tagName=tag.toUpperCase();this.style={};this.children=[];this.events=new Map();this.value='';}
  setAttribute(k,v){this[k]=v;}
  append(...items){for(const child of items){child.parent=this;this.children.push(child);}}
  remove(){if(this.parent)this.parent.children=this.parent.children.filter(c=>c!==this);}
  addEventListener(k,f){this.events.set(k,f);}
  removeEventListener(k){this.events.delete(k);}
  emit(k,event={}){this.events.get(k)?.(event);}
  focus(){assert.equal(globalThis.gesture,true,'focus must be synchronous with the user gesture');globalThis.focused=this;}
}
function environment(){
  const listeners=new Map();const viewport=new Element('viewport');viewport.height=500;viewport.offsetTop=0;
  const body=new Element('body');
  globalThis.window={innerHeight:800,visualViewport:viewport};
  globalThis.document={body,createElement:t=>new Element(t),addEventListener:(n,f)=>listeners.set(n,f),removeEventListener:(n)=>listeners.delete(n)};
  const canvas={tagName:'CANVAS',getBoundingClientRect:()=>({left:0,top:0,width:400,height:800})};
  return {body,listeners,viewport,tap(x=10,y=20,type='touch'){
    const e={target:canvas,isPrimary:true,pointerType:type,pointerId:1,clientX:x,clientY:y};
    listeners.get('pointerdown')?.(e);globalThis.gesture=true;listeners.get('pointerup')?.(e);globalThis.gesture=false;
  },canvas};
}
test('touch editing focuses a real password field, commits Unicode and clears the secret on close',()=>{
  const e=environment();const calls=[];
  const stop=installMobileInput(()=>JSON.stringify({token:1,label:'Code',value:'synthetic-secret',secret:true,multiline:false,max_length:64,input_mode:'text'}),(...args)=>{calls.push(args);return true;});
  e.tap();const panel=e.body.children[0],input=panel.children[0].children[1],done=panel.children[1];
  assert.equal(globalThis.focused,input);assert.equal(input.type,'password');assert.equal(input.autocomplete,'off');assert.equal(panel.style.bottom,'300px');
  input.value='Привет 👋';input.emit('input');assert.deepEqual(calls[0],[1,'Привет 👋',false]);
  done.emit('click');assert.deepEqual(calls[1],[1,'Привет 👋',true]);assert.equal(input.value,'');assert.equal(e.body.children.length,0);
  stop();assert.equal(e.listeners.size,0);
});
test('scrolling, cancelled pointers, desktop clicks and readonly targets do not summon an editor',()=>{
  const e=environment();let opens=0;const stop=installMobileInput(()=>{opens++;return '';},()=>true);
  e.tap(10,20,'mouse');assert.equal(opens,0);
  const down={target:e.canvas,isPrimary:true,pointerType:'touch',pointerId:1,clientX:10,clientY:20};
  e.listeners.get('pointerdown')(down);e.listeners.get('pointerup')({...down,clientY:70});assert.equal(opens,0);
  e.listeners.get('pointerdown')(down);e.listeners.get('pointercancel')();e.listeners.get('pointerup')(down);assert.equal(opens,0);
  e.tap();assert.equal(opens,1);assert.equal(e.body.children.length,0);stop();
});
test('stale fields close immediately and textarea editing uses the browser IME',()=>{
  const e=environment();const stop=installMobileInput(()=>JSON.stringify({token:2,label:'Message',value:'',secret:false,multiline:true,max_length:8192,input_mode:'text'}),()=>false);
  e.tap();const panel=e.body.children[0],input=panel.children[0].children[1];assert.equal(input.tagName,'TEXTAREA');
  e.viewport.height=300;e.viewport.offsetTop=50;e.viewport.emit('resize');assert.equal(panel.style.bottom,'450px');
  input.value='new';input.emit('input');assert.equal(input.value,'');assert.equal(e.body.children.length,0);stop();
});
