const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../public/assets/tuliprox_player.js'), 'utf8');
function testNativeFallback({ name, url, isHls, isLive, expectMpegTsFallback }) {
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
  const handle = window.attachTuliproxVideo(video, url, isHls, false, isLive, () => errors++, null);
  const errorHandler = listeners.get('error');
  assert.equal(typeof errorHandler, 'function');
  errorHandler();

  if (expectMpegTsFallback) {
    assert.equal(players.length, 1);
    assert.equal(players[0].isLive, isLive);
    assert.equal(players[0].url, url);
    assert.equal(players[0].type, 'mpegts');
    assert.equal(errors, 0);
    assert.ok(handle.mpegts);
  } else {
    assert.equal(players.length, 0);
    assert.equal(errors, 1);
    assert.equal(handle.mpegts, null);
  }

  window.detachTuliproxVideo(handle, video);
  assert.equal(destroyed, expectMpegTsFallback ? 1 : 0);
  console.log(`PASS ${name}`);
}

testNativeFallback({
  name: 'live MPEG-TS retries through the MPEG-TS engine',
  url: '/live/123.ts',
  isHls: false,
  isLive: true,
  expectMpegTsFallback: true
});
testNativeFallback({
  name: 'on-demand MPEG-TS retries through the MPEG-TS engine',
  url: '/vod/123.ts',
  isHls: false,
  isLive: false,
  expectMpegTsFallback: true
});
testNativeFallback({
  name: 'native HLS failure does not feed an HLS manifest to the MPEG-TS engine',
  url: '/live/playlist.m3u8',
  isHls: true,
  isLive: true,
  expectMpegTsFallback: false
});
