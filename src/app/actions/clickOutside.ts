export function clickOutside(node: HTMLElement, handler: () => void) {
  let pointerStartedInside = false;

  const isInside = (event: Event) =>
    event.composedPath().includes(node) || node.contains(event.target as Node);

  const onPointerDown = (event: PointerEvent) => {
    pointerStartedInside = isInside(event);
  };

  const resetPointer = () => {
    pointerStartedInside = false;
  };

  const onClick = (event: MouseEvent) => {
    // 输入框内开始的拖选可能在外部产生 click，不能将它当作主动点击外部。
    // detail 为 0 的键盘/程序触发点击不属于上一次指针操作。
    const startedInside = event.detail > 0 && pointerStartedInside;
    resetPointer();
    if (!startedInside && !isInside(event)) {
      handler();
    }
  };

  document.addEventListener('pointerdown', onPointerDown, true);
  document.addEventListener('pointercancel', resetPointer, true);
  document.addEventListener('click', onClick, true);
  window.addEventListener('blur', resetPointer);

  return {
    destroy() {
      document.removeEventListener('pointerdown', onPointerDown, true);
      document.removeEventListener('pointercancel', resetPointer, true);
      document.removeEventListener('click', onClick, true);
      window.removeEventListener('blur', resetPointer);
    },
  };
}
