import { DEFAULT_TYPOGRAPHY, type TypographyOptions } from './types';

let current: TypographyOptions = { ...DEFAULT_TYPOGRAPHY };
const listeners = new Set<() => void>();

export function getTypographyOptions(): TypographyOptions {
  return { ...current };
}

export function setTypographyOptions(value: Partial<TypographyOptions>) {
  const next = {
    enabled: value.enabled ?? current.enabled,
    profile:
      value.profile === 'zh-TW'
        ? ('zh-TW' as const)
        : value.profile === 'zh-CN'
          ? ('zh-CN' as const)
          : current.profile,
    hanging: value.hanging ?? current.hanging,
  };
  current = next;
  for (const listener of listeners) listener();
}

export function subscribeTypography(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
