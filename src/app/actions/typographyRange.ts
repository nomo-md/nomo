import {
  beginTypographyInteraction,
  type TypographyGeometrySource,
} from '../../lib/typography/interaction';

/** Keep preview across a range gesture; the first input runs before Svelte writes CSS. */
export function typographyRange(node: HTMLInputElement, initialSource: TypographyGeometrySource) {
  let source = initialSource;
  let pointerId: number | null = null;
  let previousValue = node.value;
  let release: (() => void) | undefined;
  const pressedKeys = new Set<string>();
  const adjustmentKeys = new Set([
    'ArrowLeft',
    'ArrowRight',
    'ArrowUp',
    'ArrowDown',
    'Home',
    'End',
    'PageUp',
    'PageDown',
  ]);
  const finish = () => {
    pointerId = null;
    pressedKeys.clear();
    previousValue = node.value;
    release?.();
    release = undefined;
  };
  const pointerDown = (event: PointerEvent) => {
    if (!node.disabled && event.isPrimary && event.button === 0) {
      previousValue = node.value;
      pointerId = event.pointerId;
    }
  };
  const input = () => {
    if (node.disabled || node.value === previousValue) return;
    previousValue = node.value;
    release ??= beginTypographyInteraction(source);
  };
  const pointerEnd = (event: PointerEvent) => {
    if (pointerId === event.pointerId) finish();
  };
  const change = () => {
    // Keyboard ranges may emit change on every repeated key; keyup ends that gesture.
    if (pointerId === null && !pressedKeys.size) finish();
  };
  const keyDown = (event: KeyboardEvent) => {
    if (adjustmentKeys.has(event.key)) {
      if (!pressedKeys.size) previousValue = node.value;
      pressedKeys.add(event.key);
    }
  };
  const keyUp = (event: KeyboardEvent) => {
    if (pressedKeys.delete(event.key) && !pressedKeys.size) finish();
  };
  node.addEventListener('pointerdown', pointerDown, true);
  node.addEventListener('input', input, true);
  node.addEventListener('change', change);
  node.addEventListener('keydown', keyDown, true);
  node.addEventListener('keyup', keyUp);
  node.addEventListener('blur', finish);
  window.addEventListener('pointerup', pointerEnd, true);
  window.addEventListener('pointercancel', pointerEnd, true);
  window.addEventListener('blur', finish);
  return {
    update(nextSource: TypographyGeometrySource) {
      if (nextSource !== source) {
        finish();
        source = nextSource;
      }
      previousValue = node.value;
    },
    destroy() {
      node.removeEventListener('pointerdown', pointerDown, true);
      node.removeEventListener('input', input, true);
      node.removeEventListener('change', change);
      node.removeEventListener('keydown', keyDown, true);
      node.removeEventListener('keyup', keyUp);
      node.removeEventListener('blur', finish);
      window.removeEventListener('pointerup', pointerEnd, true);
      window.removeEventListener('pointercancel', pointerEnd, true);
      window.removeEventListener('blur', finish);
      finish();
    },
  };
}
