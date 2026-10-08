import { createDomTypography, waitForTypographyAssets } from './dom';
import { DEFAULT_TYPOGRAPHY, type TypographyOptions } from './types';

declare global {
  interface Window {
    __NOMO_FLUSH_TYPOGRAPHY__?: () => Promise<void>;
    __NOMO_PREPARE_PRINT__?: (widthMm: number) => Promise<void>;
  }
}

const root = document.querySelector<HTMLElement>('.nomo-export');
if (root) {
  const options: TypographyOptions = {
    ...DEFAULT_TYPOGRAPHY,
    ...JSON.parse(document.getElementById('nomo-typography-options')?.textContent ?? '{}'),
  };
  const controller = createDomTypography(root, options);
  window.__NOMO_FLUSH_TYPOGRAPHY__ = async () => {
    await waitForTypographyAssets(root);
    controller.invalidate();
    await controller.flush();
  };
  window.__NOMO_PREPARE_PRINT__ = async (widthMm) => {
    if (!Number.isFinite(widthMm) || widthMm <= 0) throw new Error('Invalid print width');
    root.style.setProperty('width', `${widthMm}mm`, 'important');
    root.style.setProperty('max-width', 'none', 'important');
    root.style.setProperty('padding', '0', 'important');
    root.style.setProperty('margin', '0', 'important');
    await window.__NOMO_FLUSH_TYPOGRAPHY__!();
  };
  void window.__NOMO_FLUSH_TYPOGRAPHY__().catch(() => {
    root.dataset.kpState = 'error';
  });
}
