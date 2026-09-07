import { useEffect, useRef } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalSize } from "@tauri-apps/api/dpi";

const BASE_WIDTH = 420;
const MIN_HEIGHT = 300;
const MAX_HEIGHT = 900;

function getUiScale(): number {
  const fs = parseFloat(getComputedStyle(document.documentElement).fontSize);
  return fs / 13; // 13px is the base font size
}

export function useAutoResize(ref: React.RefObject<HTMLElement | null>) {
  const lastHeight = useRef(0);
  const lastWidth = useRef(0);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    const update = async () => {
      const uiScale = getUiScale();
      const width = Math.round(BASE_WIDTH * uiScale);

      // Sum each direct child's scrollHeight to get the "natural" total
      // This bypasses flex constraints and overflow on .app/.content
      let natural = 0;
      for (const child of el.children) {
        natural += child.scrollHeight;
      }

      // The Liquid Glass canvas reserves transparent space around its panes.
      // Include that space (and the flex gap) so the native window does not
      // stop growing before the content has reached the same usable height as
      // the Cobalt theme.
      const styles = getComputedStyle(el);
      const paddingY = parseFloat(styles.paddingTop) + parseFloat(styles.paddingBottom);
      const rowGap = parseFloat(styles.rowGap) || 0;
      const gaps = Math.max(0, el.children.length - 1) * rowGap;
      natural += paddingY + gaps;

      const isLiquidGlass = document.documentElement.dataset.theme === "liquid-glass";
      const clamped = isLiquidGlass
        ? MAX_HEIGHT
        : Math.max(MIN_HEIGHT, Math.min(MAX_HEIGHT, natural + 10));

      const heightChanged = Math.abs(clamped - lastHeight.current) > 2;
      const widthChanged = width !== lastWidth.current;
      if (!heightChanged && !widthChanged) return;

      try {
        const win = getCurrentWebviewWindow();
        await win.setSize(new LogicalSize(width, clamped));
        lastHeight.current = clamped;
        lastWidth.current = width;
      } catch {
        // ignore — window may not be ready
      }
    };

    // ResizeObserver catches layout shifts (expand/collapse)
    const observer = new ResizeObserver(() => update());
    observer.observe(el);

    // MutationObserver catches DOM changes (tab switch, data fetch, settings toggle)
    const mutObserver = new MutationObserver(() => update());
    mutObserver.observe(el, { childList: true, subtree: true });

    // Theme is applied to <html>, outside the observed app subtree. Without
    // this observer, switching to Liquid Glass could leave the window at the
    // height calculated for Cobalt until some unrelated content changed.
    const themeObserver = new MutationObserver(() => update());
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });

    update();

    return () => {
      observer.disconnect();
      mutObserver.disconnect();
      themeObserver.disconnect();
    };
  }, [ref]);
}
