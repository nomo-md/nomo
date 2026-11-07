import test from 'node:test';
import assert from 'node:assert/strict';
import { analyzeOpeningPixels } from './opening.mjs';

function recording({ blank = false, echo = true } = {}) {
  const width = 80, height = 50, size = width * height;
  const baseline = Buffer.alloc(size, 240), complete = Buffer.alloc(size, 240);
  for (let y = 10; y < 16; y++) for (let x = 10; x < 20; x++) baseline[y * width + x] = 20;
  if (!blank) for (let y = 7; y < 13; y++) for (let x = 25; x < 35; x++) complete[y * width + x] = 30;
  const input = Buffer.from(complete);
  if (echo) for (let y = 7; y < 11; y++) for (let x = 37; x < 40; x++) input[y * width + x] = 30;
  const partial = Buffer.from(baseline);
  for (let y = 7; y < 10; y++) for (let x = 25; x < 35; x++) partial[y * width + x] = 30;
  return {
    raw: Buffer.concat(Array.from({ length: 32 }, (_, i) => i < 11 ? baseline : i === 11 ? partial : i < 25 ? complete : input)),
    capture: { frames: Array.from({ length: 32 }, (_, i) => [i * 25, i * 25 + 4]), width, height, fullWidth: 320, fullHeight: 200 },
    timing: { started: 150, offset: 0, uncertaintyMs: .2, baseline: 5, complete: 24, echo: 31, readableMs: 200, readyMs: 300, inputMs: 460 },
    bounds: { x: 0, y: 0, width: 320, height: 200 }, blank,
  };
}
const analyze = f => analyzeOpeningPixels(f.raw, f.capture, f.timing, f.bounds, 1, f.blank);
test('旧文档与局部像素不能充当目标首屏，真实回显必须晚于按键', () => {
  const value = analyze(recording());
  assert.equal(value.firstVisibleMs, 154);
  assert.equal(value.completeInteractiveMs, 304);
  assert.equal(value.inputEchoMs, 479);
  assert.ok(value.inputEchoMs > value.inputSentMs);
  assert.equal(value.confirmationOverheadMs, 175);
  assert.deepEqual(value.firstVisibleBracketMs, [125, 154]);
});
test('空白文档同时要求身份就绪与等待页消失', () => {
  const value = analyze(recording({ blank: true }));
  assert.equal(value.firstVisibleMs, 204);
});
test('正文接受按键但屏幕未改变不能通过', () => {
  assert.throws(() => analyze(recording({ echo: false })), /未取得可区分/);
});
test('失焦、截断像素流和越界视口必须使测量失败', () => {
  const f = recording();
  f.capture.failure = 'foreground window changed';
  assert.throws(() => analyze(f), /foreground/);
  delete f.capture.failure;
  f.raw = f.raw.subarray(1);
  assert.throws(() => analyze(f), /长度不匹配/);
  const other = recording(); other.bounds.x = -50;
  assert.throws(() => analyze(other), /不在捕获客户区/);
});
test('未稳定的首屏不能只凭末帧判为完整', () => {
  const f = recording();
  f.timing.complete = 13;
  assert.throws(() => analyze(f), /稳定画面/);
});
test('按键准备造成的焦点或选区变化不抬高自然首屏完成时间', () => {
  const f = recording(), size = f.capture.width * f.capture.height;
  f.timing.complete = 18;
  f.timing.preInput = 24;
  for (let i = 19; i < 32; i++) for (let y = 20; y < 30; y++) for (let x = 45; x < 55; x++)
    f.raw[i * size + y * f.capture.width + x] = 30;
  const value = analyze(f);
  assert.equal(value.contentCompleteMs, 154);
  assert.equal(value.completeInteractiveMs, 304);
  assert.equal(value.inputEchoMs, 479);
});
