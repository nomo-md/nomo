import type { OpenTarget, OpenTargetRouteDecision } from './desktopWindow';
import type { OpenDefaultBehavior } from './settings';

export interface OpenTargetRouting {
  syncTargets(): Promise<void>;
  prepare(target: OpenTarget, create: boolean): Promise<OpenTargetRouteDecision>;
  activateCurrent(): Promise<void>;
  openCurrent(target: OpenTarget): Promise<void>;
  createWindow(label: string): Promise<void>;
  isReusableInitialWindow(): boolean;
  getBehavior(): OpenDefaultBehavior;
  requestChoice(target: OpenTarget): Promise<{
    choice: 'current-window' | 'new-window';
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
      const result = await routing.requestChoice(decision.target);
      if (!result) return;
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
    // 先完成当前目录的标签打开，再为批次中未匹配目录的文件选择去向。
    await routeOpenTarget(decision.remainingTarget, routing);
  }
}
