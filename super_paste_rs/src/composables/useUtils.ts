import { onMounted, onUnmounted } from 'vue';

/**
 * 监听键盘快捷键
 */
export function useKeyboard(handlers: Record<string, (e: KeyboardEvent) => void>) {
  const handleKeydown = (e: KeyboardEvent) => {
    const key = [
      e.ctrlKey && 'Ctrl',
      e.shiftKey && 'Shift',
      e.altKey && 'Alt',
      e.metaKey && 'Meta',
      e.key,
    ]
      .filter(Boolean)
      .join('+');

    if (handlers[key]) {
      e.preventDefault();
      handlers[key](e);
    }
  };

  onMounted(() => {
    window.addEventListener('keydown', handleKeydown);
  });

  onUnmounted(() => {
    window.removeEventListener('keydown', handleKeydown);
  });
}

/**
 * 防抖
 */
export function useDebounce<T extends (...args: any[]) => any>(
  fn: T,
  delay: number = 300
): (...args: Parameters<T>) => void {
  let timer: ReturnType<typeof setTimeout> | null = null;

  return (...args: Parameters<T>) => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => fn(...args), delay);
  };
}

/**
 * 节流
 */
export function useThrottle<T extends (...args: any[]) => any>(
  fn: T,
  delay: number = 300
): (...args: Parameters<T>) => void {
  let lastCall = 0;

  return (...args: Parameters<T>) => {
    const now = Date.now();
    if (now - lastCall >= delay) {
      lastCall = now;
      fn(...args);
    }
  };
}

/**
 * 剪贴板操作
 */
export function useClipboard() {
  const copy = async (text: string): Promise<boolean> => {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch (error) {
      console.error('复制失败:', error);
      return false;
    }
  };

  const read = async (): Promise<string | null> => {
    try {
      return await navigator.clipboard.readText();
    } catch (error) {
      console.error('读取剪贴板失败:', error);
      return null;
    }
  };

  return { copy, read };
}
