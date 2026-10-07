import type { OpenDirectoryWindow, OpenTarget, OpenTargetRouteDecision } from './desktopWindow';
import type { OpenDefaultBehavior } from './settings';

export interface OpenTargetRouting {
  syncTargets(): Promise<void>;
  prepare(target: OpenTarget, create: boolean): Promise<OpenTargetRouteDecision>;
  activateCurrent(): Promise<void>;
  openCurrent(target: OpenTarget): Promise<void>;
  createWindow(label: string): Promise<void>;
  isReusableInitialWindow(): boolean;
  getBehavior(): OpenDefaultBehavior;
  openDirectory?(candidate: OpenDirectoryWindow): Promise<void>;
  requestChoice(
    target: OpenTarget,
    directoryWindows?: OpenDirectoryWindow[],
  ): Promise<{
    choice: 'current-window' | 'new-window' | 'directory-window';
    windowLabel?: string;
    remember: boolean;
  } | null>;
  rememberBehavior(behavior: 'current-window' | 'new-window'): Promise<void>;
}

/** 保留已有打开策略，仅在确定使用当前窗口或需要询问时激活接收窗口。 */
export async function routeOpenTarget(
  target: OpenTarget,
  routing: OpenTargetRouting,
): Promise<void> {
  await routing.syncTargets();
  let decision = await routing.prepare(target, false);
  if (decision.action === 'handled') return;

  let activated = false;
  let useCurrent = decision.action === 'activate-current' || routing.isReusableInitialWindow();
  if (!useCurrent) {
    const behavior = routing.getBehavior();
    useCurrent = behavior === 'current-window';
    if (behavior === 'ask-every-time') {
      await routing.activateCurrent();
      activated = true;
      const result = await routing.requestChoice(
        decision.target,
        decision.action === 'open-current' ? decision.directoryWindows : undefined,
      );
      if (!result) return;
      if (result.choice === 'directory-window') {
        const candidate =
          decision.action === 'open-current'
            ? decision.directoryWindows?.find((item) => item.windowLabel === result.windowLabel)
            : undefined;
        if (!candidate || !routing.openDirectory)
          throw new Error('Selected directory window is unavailable');
        await routing.openDirectory(candidate);
        if (decision.target.kind === 'documents') {
          const paths = decision.target.paths.filter((path) => !candidate.paths.includes(path));
          if (paths.length > 0) await routeOpenTarget({ kind: 'documents', paths }, routing);
        }
        return;
      }
      if (result.remember) await routing.rememberBehavior(result.choice);
      useCurrent = result.choice === 'current-window';
    }
  }

  if (!useCurrent) {
    decision = await routing.prepare(decision.target, true);
    if (decision.action === 'handled') return;
    if (decision.action === 'create-window') {
      await routing.createWindow(decision.windowLabel);
      return;
    }
  }

  if (!activated) await routing.activateCurrent();
  await routing.openCurrent(decision.target);
  if (decision.action === 'activate-current' && decision.remainingTarget) {
    // 先定位当前窗口已打开的标签，再为批次中的未打开文件选择去向。
    await routeOpenTarget(decision.remainingTarget, routing);
  }
}
