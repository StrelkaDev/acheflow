import test from 'node:test';
import assert from 'node:assert/strict';
import {listMonitorOutputs, openMonitor, requestMonitorOutputAccess} from './audio_monitor.js';

test('Web Audio monitor plays stereo PCM at a bounded live delay and closes its output', async () => {
  let context;
  globalThis.AudioContext = class {
    constructor() {
      context = this;
      this.currentTime = 10;
      this.destination = {};
      this.gain = {gain:{value: 1}, connect() {}, disconnect() { this.disconnected = true; }};
      this.buffers = [];
      this.starts = [];
    }
    createGain() { return this.gain; }
    resume() { return Promise.resolve(); }
    createBuffer(channels, frames, rate) {
      const data = Array.from({length: channels}, () => new Float32Array(frames));
      this.buffers.push(data);
      return {duration: frames / rate, getChannelData: channel => data[channel]};
    }
    createBufferSource() {
      const starts = this.starts;
      return {connect() {}, disconnect() {}, start: at => starts.push(at)};
    }
    close() { this.closed = true; return Promise.resolve(); }
  };

  const playback = openMonitor(48000);
  await playback.ready;
  const buffer = new Uint8Array(4800 * 4);
  buffer.set([0, 64, 0, 128, 0, 192, 0, 32]);
  for (let i = 0; i < 6; i++) playback.play(buffer, 0.25);
  assert.deepEqual([...context.buffers[0][0].slice(0, 2)], [0.5, -0.5]);
  assert.deepEqual([...context.buffers[0][1].slice(0, 2)], [-1, 0.25]);
  assert.equal(context.gain.gain.value, 0.25);
  assert.equal(context.starts.length, 2);
  context.currentTime = 20;
  playback.play(buffer, 0.75);
  assert.equal(context.starts.at(-1), 20.06);
  playback.close();
  assert.equal(context.gain.disconnected, true);
  assert.equal(context.closed, true);
});

test('a selected output waits for routing and reports a disconnected device', async () => {
  let requestedSink;
  let rejectRouting;
  let closed = false;
  globalThis.AudioContext = class {
    destination = {};
    createGain() { return {connect() {}, disconnect() {}}; }
    resume() { return Promise.resolve(); }
    setSinkId(id) {
      requestedSink = id;
      return new Promise((_, reject) => { rejectRouting = reject; });
    }
    close() { closed = true; return Promise.resolve(); }
  };

  const playback = openMonitor(48000, 'headphones');
  assert.equal(requestedSink, 'headphones');
  rejectRouting(new Error('Device unavailable'));
  await assert.rejects(playback.ready, /selected audio output/);
  playback.close();
  assert.equal(closed, true);
});

test('media element routing uses the chosen output when AudioContext cannot select a sink', async () => {
  let element;
  let connectedTo;
  let trackStopped = false;
  const destination = {stream: {getTracks: () => [{stop() { trackStopped = true; }}]}};
  globalThis.AudioContext = class {
    createGain() { return {connect(target) { connectedTo = target; }, disconnect() {}}; }
    createMediaStreamDestination() { return destination; }
    resume() { return Promise.resolve(); }
    close() { return Promise.resolve(); }
  };
  globalThis.Audio = class {
    constructor() { element = this; }
    setSinkId(id) { this.sinkId = id; return Promise.resolve(); }
    play() { return Promise.resolve(); }
    pause() { this.paused = true; }
  };
  const playback = openMonitor(48000, 'usb');
  await playback.ready;
  assert.equal(connectedTo, destination);
  assert.equal(element.sinkId, 'usb');
  assert.equal(element.srcObject, destination.stream);
  playback.close();
  assert.equal(element.srcObject, null);
  assert.equal(element.paused, true);
  assert.equal(trackStopped, true);
});

test('browser discovery hides default aliases and releases the temporary microphone', async () => {
  globalThis.AudioContext = class { setSinkId() {} };
  let permitted = false;
  let microphoneStopped = false;
  Object.defineProperty(globalThis, 'navigator', {configurable: true, value: {
    mediaDevices: {
      async enumerateDevices() {
        return [
          {kind: 'audiooutput', deviceId: 'default'},
          ...(permitted ? [{kind: 'audiooutput', deviceId: 'usb', label: 'USB Headphones'}] : []),
        ];
      },
      async getUserMedia() {
        permitted = true;
        return {getTracks: () => [{stop() { microphoneStopped = true; }}]};
      },
    },
  }});
  assert.equal(JSON.parse(await listMonitorOutputs()).action, 'discover');
  await requestMonitorOutputAccess(false);
  assert.equal(microphoneStopped, true);
  assert.deepEqual(JSON.parse(await listMonitorOutputs()).devices, [{id: 'usb', label: 'USB Headphones'}]);
});
