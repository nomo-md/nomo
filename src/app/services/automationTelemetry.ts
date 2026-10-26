/** 仅测试构建导入。采样以有界直方图汇总，避免长时间测试不断累积事件。 */
export function createAutomationTelemetry() {
  const frameBins = new Uint32Array(1001);
  const inputBins = new Uint32Array(1001);
  let frames = 0, frameMax = 0, longTasks = 0, longTaskMs = 0, inputEvents = 0;
  let lastFrame = performance.now(), frame = 0, stopped = false;
  let inputs: { at: number; root: Element }[] = [];
  let unmatchedInputs = 0;
  const record = (bins: Uint32Array, ms: number) => bins[Math.min(1000, Math.max(0, Math.round(ms)))]++;
  const onFrame = (now: number) => {
    if (stopped) return;
    const delta = now - lastFrame; lastFrame = now;
    const expired = inputs.filter(input => now - input.at > 5000);
    unmatchedInputs += expired.length; inputs = inputs.filter(input => now - input.at <= 5000);
    record(frameBins, delta); frames++; frameMax = Math.max(frameMax, delta);
    frame = requestAnimationFrame(onFrame);
  };
  const beforeInput = (event: Event) => {
    const root = event.target instanceof Element ? event.target.closest('.ProseMirror,.cm-editor') : null;
    if (!root || (event instanceof InputEvent && event.isComposing)) return;
    if (inputs.length >= 512) { inputs.shift(); unmatchedInputs++; }
    inputs.push({ at: performance.now(), root }); inputEvents++;
  };
  const mutation = new MutationObserver(records => {
    const matching = inputs.filter(input => records.some(record => input.root.contains(record.target)));
    if (!matching.length) return;
    const matched = new Set(matching); inputs = inputs.filter(input => !matched.has(input));
    requestAnimationFrame(() => { if (!stopped) for (const pending of matching) record(inputBins, performance.now() - pending.at); });
  });
  mutation.observe(document, { subtree: true, childList: true, characterData: true });
  document.addEventListener('beforeinput', beforeInput, true);
  let observer: PerformanceObserver | undefined;
  if (PerformanceObserver.supportedEntryTypes.includes('longtask')) {
    observer = new PerformanceObserver(list => { for (const e of list.getEntries()) { longTasks++; longTaskMs += e.duration; } });
    observer.observe({ type: 'longtask' });
  }
  frame = requestAnimationFrame(onFrame);
  function histogram(bins: Uint32Array) {
    const count = bins.reduce((a, b) => a + b, 0);
    const quantile = (ratio: number) => { let sum = 0; for (let i = 0; i < bins.length; i++) { sum += bins[i]; if (sum >= Math.ceil(count * ratio)) return i; } return null; };
    return { count, p50Ms: count ? quantile(.5) : null, p95Ms: count >= 20 ? quantile(.95) : null,
      p99Ms: count >= 100 ? quantile(.99) : null, overflowAt1000Ms: bins[1000] };
  }
  return {
    read: () => ({ frames, frameMaxMs: frameMax, frameInterval: histogram(frameBins), inputEvents, unmatchedInputs, pendingInputs: inputs.length,
      inputToFrameEstimate: histogram(inputBins), longTasks: observer ? longTasks : null,
      longTaskMs: observer ? longTaskMs : null, timeOrigin: performance.timeOrigin, now: performance.now() }),
    dispose: () => { stopped = true; cancelAnimationFrame(frame); observer?.disconnect(); mutation.disconnect();
      document.removeEventListener('beforeinput', beforeInput, true); inputs = []; },
  };
}
