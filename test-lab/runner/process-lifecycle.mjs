export async function stopOwnedProcess(child, { terminate, timeoutMs = 5000, graceMs = 0 } = {}) {
  if (!child) return null;
  const wait = async ms => { let timer; try { return await Promise.race([child.finished, new Promise(resolve => { timer = setTimeout(() => resolve(null), ms); })]); } finally { clearTimeout(timer); } };
  let result = await wait(graceMs);
  if (result) return result;
  let terminationError;
  try { await terminate(child); } catch (error) { terminationError = error; }
  result = await wait(timeoutMs);
  if (!result) throw new Error(`无法确认测试进程 ${child.pid} 已退出${terminationError ? `：${terminationError}` : ''}`);
  return result;
}
