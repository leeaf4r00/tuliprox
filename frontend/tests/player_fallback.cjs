const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../public/assets/tuliprox_player.js'), 'utf8');
for (const live of [true, false]) {
  const listeners = new Map();
  const players = [];
  let errors = 0;
  let destroyed = 0;
  const video = {
    currentTime: 0, paused: true, error: { code: 4 },
    addEventListener(name, fn) { listeners.set(name, fn); },
    removeEventListener(name, fn) { if (listeners.get(name) === fn) listeners.delete(name); },
    pause() {}, load() {}, removeAttribute() {}
  };
  const window = { mpegts: {
    isSupported: () => true,
    Events: { ERROR: 'error' },
    createPlayer(options) {
      players.push(options);
      return { on() {}, attachMediaElement() {}, load() {}, play() {}, destroy() { destroyed++; } };
    }
  } };
  vm.runInNewContext(source, { window, setTimeout, clearTimeout });
  const handle = window.attachTuliproxVideo(video, '/live/123', false, false, live, () => errors++, null);
  listeners.get('error')();
  assert.equal(players.length, 1);
  assert.equal(players[0].isLive, live);
  assert.equal(players[0].url, '/live/123');
  assert.equal(players[0].type, 'mpegts');
  assert.equal(errors, 0);
  assert.ok(handle.mpegts);
  window.detachTuliproxVideo(handle, video);
  assert.equal(destroyed, 1);
  console.log(`PASS native fallback: live=${live}, playback mode and cleanup preserved`);
}

{
  const listeners = new Map();
  let fallbackAttempts = 0;
  let errors = 0;
  const video = {
    currentTime: 0, paused: true, error: { code: 4 },
    addEventListener(name, fn) { listeners.set(name, fn); },
    removeEventListener(name, fn) { if (listeners.get(name) === fn) listeners.delete(name); },
    pause() {}, load() {}, removeAttribute() {}
  };
  const window = { mpegts: {
    isSupported: () => true,
    createPlayer() { fallbackAttempts++; throw new Error('HLS must not use MPEG-TS fallback'); }
  } };
  vm.runInNewContext(source, { window, setTimeout, clearTimeout });
  const handle = window.attachTuliproxVideo(video, '/live/123.m3u8', true, false, true, () => errors++, null);
  listeners.get('error')();
  assert.equal(fallbackAttempts, 0);
  assert.equal(errors, 1);
  window.detachTuliproxVideo(handle, video);
  console.log('PASS unsupported HLS does not enter MPEG-TS fallback');
}
