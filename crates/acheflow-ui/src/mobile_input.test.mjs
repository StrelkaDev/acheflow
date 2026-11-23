import test from 'node:test';
import assert from 'node:assert/strict';
import {installTextInput} from './mobile_input.js';

const box = {x: 20, y: 70, width: 180, height: 38};
const descriptor = (overrides = {}) => JSON.stringify({
  label: 'Destination name', placeholder: '', value: 'Initial name', rect: box,
  secret: false, multiline: false, code: false, max_length: 64,
  input_mode: 'text', start: 2, end: 5, padding: {top: 11, right: 13, bottom: 11, left: 13}, ...overrides,
});

class Element {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.style = {};
    this.children = [];
    this.events = new Map();
    this.value = '';
  }
  setAttribute(key, value) { this[key] = value; }
  append(child) { child.parent = this; this.children.push(child); }
  remove() { this.parent.children = this.parent.children.filter(child => child !== this); }
  addEventListener(name, fn) { this.events.set(name, fn); }
  emit(name, extra = {}) {
    this.events.get(name)?.({stopPropagation() {}, preventDefault() {}, ...extra});
  }
  focus() { globalThis.focused = this; }
  setSelectionRange(start, end) { this.selection = [start, end]; }
}

function environment() {
  const listeners = new Map();
  const frames = new Map();
  let nextFrame = 0;
  const body = new Element('body');
  const viewport = {width: 400, height: 600, offsetLeft: 0, offsetTop: 0};
  globalThis.window = {visualViewport: viewport};
  globalThis.document = {
    body, createElement: tag => new Element(tag),
    addEventListener: (name, fn) => listeners.set(name, fn),
    removeEventListener: name => listeners.delete(name),
  };
  globalThis.requestAnimationFrame = fn => {frames.set(++nextFrame, fn); return nextFrame;};
  globalThis.cancelAnimationFrame = id => frames.delete(id);
  const canvas = new Element('canvas');
  canvas.clientWidth = 400;
  canvas.clientHeight = 600;
  canvas.getBoundingClientRect = () => ({left: 5, top: 10, width: 400, height: 600});
  return {
    body, canvas, listeners, viewport,
    tick() {
      const callbacks = [...frames.values()];
      frames.clear();
      callbacks.forEach(fn => fn());
    },
    tap(type = 'mouse', distance = 0) {
      const event = {target: canvas, isPrimary: true, pointerType: type, pointerId: 1, button: 0, clientX: 25, clientY: 80};
      listeners.get('pointerdown')?.(event);
      listeners.get('pointerup')?.({...event, clientY: 80 + distance});
      return body.children[0];
    },
  };
}

test('mouse and touch focus a real password input at the field with its existing selection', () => {
  for (const type of ['mouse', 'touch']) {
    const env = environment();
    const events = [];
    const stop = installTextInput(
      () => descriptor({value: 'fixture', secret: true}), () => JSON.stringify(box),
      (...args) => {events.push(args); return true;},
    );
    const input = env.tap(type);
    assert.equal(globalThis.focused, input);
    assert.equal(input.type, 'password');
    assert.equal(input.autocomplete, 'off');
    assert.equal(input.style.left, '25px');
    assert.equal(input.style.top, '80px');
    assert.equal(input.style.padding, '11px 13px 11px 13px');
    assert.deepEqual(input.selection, [2, 5]);
    assert.equal(input.events.has('contextmenu'), false, "the browser owns the editor's context menu");
    input.value = 'Привет 👋';
    input.emit('input');
    input.emit('blur');
    assert.deepEqual(events, [['input', 'Привет 👋'], ['blur', 'Привет 👋']]);
    assert.equal(input.value, '');
    assert.equal(env.body.children.length, 0);
    stop();
  }
});

test('touch scrolling does not open a field, and editor geometry follows scrolling and the keyboard', () => {
  const env = environment();
  let rect = box;
  const stop = installTextInput(() => descriptor({multiline: true}), () => JSON.stringify(rect), () => true);
  assert.equal(env.tap('touch', 50), undefined);
  const input = env.tap('touch');
  assert.equal(input.tagName, 'TEXTAREA');
  rect = {...box, y: 420, height: 96};
  env.viewport.height = 320;
  env.tick();
  assert.equal(input.style.top, '224px');
  assert.equal(input.style.height, '96px');
  stop();
  assert.equal(env.listeners.size, 0);
});

test('Tab transfers editing to the next field and Escape restores the original value', () => {
  const env = environment();
  const events = [];
  let index = 0;
  const stop = installTextInput(
    () => descriptor({value: `Field ${++index}`}), () => JSON.stringify(box),
    (...args) => {events.push(args); return true;},
  );
  const first = env.tap();
  first.value = 'new';
  first.emit('keydown', {key: 'Tab'});
  assert.deepEqual(events, [['input', 'new'], ['tab', 'new']]);
  const second = env.body.children[0];
  second.value = 'edited';
  second.emit('input');
  second.emit('keydown', {key: 'Escape'});
  assert.deepEqual(events.slice(2), [['input', 'edited'], ['input', 'Field 2'], ['escape', 'Field 2']]);
  first.emit('blur');
  assert.equal(events.length, 5);
  assert.equal(env.body.children.length, 0);
  stop();
});

test('IME updates the app when composition completes and keeps Enter inside multiline editors', () => {
  const env = environment();
  const events = [];
  const stop = installTextInput(
    () => descriptor({multiline: true}), () => JSON.stringify(box),
    (...args) => {events.push(args); return true;},
  );
  const input = env.tap();
  input.emit('compositionstart');
  input.value = '日';
  input.emit('input', {isComposing: true});
  input.emit('keydown', {key: 'Enter'});
  assert.deepEqual(events, []);
  input.value = '日本語';
  input.emit('compositionend');
  input.emit('input');
  input.emit('keydown', {key: 'Enter'});
  assert.deepEqual(events, [['input', '日本語']]);
  assert.equal(env.body.children[0], input);
  input.emit('blur');
  assert.deepEqual(events[1], ['blur', '日本語']);
  stop();
});

test('a field removed or covered during editing drops the detached platform editor', () => {
  const env = environment();
  const events = [];
  const stop = installTextInput(() => descriptor(), () => '',
    (...args) => {events.push(args); return true;});
  const input = env.tap();
  env.tick();
  assert.deepEqual(events, [['blur', 'Initial name']]);
  assert.equal(input.value, '');
  assert.equal(env.body.children.length, 0);
  stop();
});

test('a zoomed visual viewport bounds the editor on both axes', () => {
  const env = environment();
  let rect = {...box, x: 310, y: 20, width: 300, height: 260};
  const stop = installTextInput(() => descriptor({rect}), () => JSON.stringify(rect), () => true);
  const input = env.tap();
  Object.assign(env.viewport, {offsetLeft: 110, offsetTop: 90, width: 180, height: 140});
  env.tick();
  assert.deepEqual(input.style, {padding: '11px 13px 11px 13px', left: '110px', top: '90px', width: '180px', height: '140px'});
  rect = {...box, x: 250, y: 180, width: 80, height: 30};
  env.tick();
  assert.equal(input.style.left, '210px');
  assert.equal(input.style.top, '190px');
  stop();
});

test('an interrupted composition preserves confirmed text and modified Tab remains a browser shortcut', () => {
  const env = environment();
  const events = [];
  const stop = installTextInput(
    () => descriptor(), () => '',
    (...args) => {events.push(args); return true;},
  );
  const input = env.tap();
  input.value = '確定';
  input.emit('input');
  input.emit('keydown', {key: 'Tab', ctrlKey: true, preventDefault() { assert.fail('browser shortcut intercepted'); }});
  input.emit('compositionstart');
  input.value = '確定日';
  input.emit('input');
  input.emit('keydown', {key: 'Enter', preventDefault() { assert.fail('composition command intercepted'); }});
  env.tick();
  assert.deepEqual(events, [['input', '確定'], ['blur', '確定']]);
  assert.equal(env.body.children.length, 0);
  stop();
});
