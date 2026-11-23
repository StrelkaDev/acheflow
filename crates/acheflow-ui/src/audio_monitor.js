export async function listMonitorOutputs() {
  const media = globalThis.navigator?.mediaDevices;
  const canRoute = typeof globalThis.AudioContext?.prototype.setSinkId === 'function'
    || typeof globalThis.HTMLMediaElement?.prototype.setSinkId === 'function';
  if (!media?.enumerateDevices || !canRoute) {
    return JSON.stringify({
      devices: [],
      action: null,
      message: 'This browser session uses the system audio output. Device selection requires a secure origin and browser support.',
    });
  }
  const outputs = await media.enumerateDevices();
  const devices = outputs
    .filter(device => device.kind === 'audiooutput' && device.deviceId
      && !['default', 'communications'].includes(device.deviceId))
    .map((device, index) => ({id: device.deviceId, label: device.label || `Audio output ${index + 1}`}));
  const action = typeof media.selectAudioOutput === 'function' ? 'choose'
    : !devices.length && typeof media.getUserMedia === 'function' ? 'discover'
    : null;
  return JSON.stringify({
    devices,
    action,
    message: action === 'discover'
      ? 'This browser needs temporary microphone access to list playback devices. Acheflow immediately releases the microphone.'
      : devices.length ? null : 'The browser has not exposed any specific audio outputs.',
  });
}

export async function requestMonitorOutputAccess(choose) {
  const media = navigator.mediaDevices;
  try {
    if (choose) {
      const device = await media.selectAudioOutput();
      return JSON.stringify({id: device.deviceId, label: device.label || 'Selected audio output'});
    }
    const stream = await media.getUserMedia({ audio: true });
    for (const track of stream.getTracks()) track.stop();
    return 'null';
  } catch (error) {
    if (choose && (error.name === 'NotAllowedError' || error.name === 'AbortError')) return 'null';
    throw 'The browser could not grant access to audio output devices.';
  }
}

// Schedule short PCM buffers only after the selected output has opened.
export function openMonitor(rate, sinkId = '') {
  const context = new AudioContext();
  const gain = context.createGain();
  let element;
  let stream;
  const close = () => {
    gain.disconnect();
    if (element) {
      element.pause();
      element.srcObject = null;
    }
    if (stream) for (const track of stream.getTracks()) track.stop();
    void context.close().catch(() => {});
  };
  let ready;
  try {
    if (!sinkId || typeof context.setSinkId === 'function') {
      gain.connect(context.destination);
      const route = sinkId ? context.setSinkId(sinkId) : Promise.resolve();
      ready = Promise.all([route, context.resume()]);
    } else {
      element = new Audio();
      if (typeof element.setSinkId !== 'function') {
        throw 'This browser cannot open the selected output. Choose System default in Settings.';
      }
      const destination = context.createMediaStreamDestination();
      stream = destination.stream;
      element.srcObject = stream;
      gain.connect(destination);
      ready = Promise.all([element.setSinkId(sinkId), context.resume(), element.play()]);
    }
  } catch (error) {
    close();
    throw error;
  }
  ready = ready.catch(() => {
    throw sinkId
      ? 'Cannot play through the selected audio output. Check the device and browser access in Settings.'
      : 'The browser blocked audio playback.';
  });
  // A monitor can be cancelled before Rust attaches to the promise.
  void ready.catch(() => {});
  let next = 0;

  return {
    ready,
    play(bytes, volume) {
      gain.gain.value = volume;
      const now = context.currentTime;
      if (next - now > 0.25) return;
      const pcm = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
      const frames = bytes.length / 4;
      const buffer = context.createBuffer(2, frames, rate);
      for (let channel = 0; channel < 2; channel++) {
        const output = buffer.getChannelData(channel);
        for (let frame = 0; frame < frames; frame++) {
          output[frame] = pcm.getInt16(frame * 4 + channel * 2, true) / 32768;
        }
      }
      const source = context.createBufferSource();
      source.buffer = buffer;
      source.connect(gain);
      source.onended = () => source.disconnect();
      if (next < now) next = now + 0.06;
      source.start(next);
      next += buffer.duration;
    },
    close,
  };
}
