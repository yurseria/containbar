import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  GlassMaterialVariant,
  isGlassSupported,
  setLiquidGlassEffect,
} from "tauri-plugin-liquid-glass-api";

export type AppTheme = "cobalt" | "liquid-glass";

export interface ThemeResult {
  nativeGlass: boolean;
}

interface LiquidGlassRegion {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
  cornerRadius: number;
  variant: number;
}

interface RegionPreset {
  selector: string;
  cornerRadius: number;
  variant: number;
}

const REGION_PRESETS: RegionPreset[] = [
  { selector: ".chrome-glass-surface", cornerRadius: 20, variant: -1 },
  { selector: ".glass-surface:not(.chrome-glass-surface)", cornerRadius: 28, variant: 8 },
  { selector: ".search-bar", cornerRadius: 20, variant: 1 },
  { selector: ".group", cornerRadius: 28, variant: 8 },
  { selector: ".settings-header", cornerRadius: 28, variant: 8 },
  { selector: ".settings-group", cornerRadius: 28, variant: 8 },
  { selector: ".runtime-card", cornerRadius: 28, variant: 8 },
  { selector: ".disconnected", cornerRadius: 28, variant: 8 },
  { selector: ".modal-dialog", cornerRadius: 20, variant: 23 },
  { selector: ".confirm-dialog", cornerRadius: 20, variant: 23 },
  { selector: ".context-menu", cornerRadius: 20, variant: 23 },
  { selector: ".error-banner", cornerRadius: 28, variant: 8 },
  { selector: ".empty", cornerRadius: 20, variant: 1 },
];

const regionIds = new WeakMap<Element, string>();
let nextRegionId = 1;
let nativeRegionGlassEnabled = false;
let regionSyncInstalled = false;
let regionSyncFrame = 0;

export function applyThemeClass(theme: AppTheme) {
  document.documentElement.dataset.theme = theme;
}

function elementRegionId(element: Element) {
  const existing = regionIds.get(element);
  if (existing) return existing;

  const id = `glass-${nextRegionId++}`;
  regionIds.set(element, id);
  return id;
}

function collectLiquidGlassRegions(): LiquidGlassRegion[] {
  const candidates = REGION_PRESETS.flatMap((preset) =>
    Array.from(document.querySelectorAll<HTMLElement>(preset.selector), (element) => ({
      element,
      preset,
    })),
  ).filter(({ element }) => {
    const style = window.getComputedStyle(element);
    return style.display !== "none" && style.visibility !== "hidden" && Number(style.opacity) > 0;
  });

  return candidates
    .filter(({ element }) =>
      !candidates.some(
        ({ element: possibleChild }) =>
          possibleChild !== element && element.contains(possibleChild),
      ),
    )
    .map(({ element, preset }) => {
      const rect = element.getBoundingClientRect();
      const left = Math.max(0, rect.left);
      const top = Math.max(0, rect.top);
      const right = Math.min(window.innerWidth, rect.right);
      const bottom = Math.min(window.innerHeight, rect.bottom);

      return {
        id: elementRegionId(element),
        x: left,
        y: top,
        width: Math.max(0, right - left),
        height: Math.max(0, bottom - top),
        cornerRadius: element.matches(".settings-runtime .runtime-card")
          ? 10
          : preset.cornerRadius,
        variant: preset.variant,
      };
    })
    .filter((region) => region.width >= 2 && region.height >= 2)
    .slice(0, 48);
}

function scheduleNativeGlassRegionSync() {
  if (!nativeRegionGlassEnabled || regionSyncFrame !== 0) return;

  regionSyncFrame = window.requestAnimationFrame(() => {
    regionSyncFrame = 0;
    if (!nativeRegionGlassEnabled) return;

    void invoke("set_liquid_glass_regions", {
      regions: collectLiquidGlassRegions(),
    }).catch((error) => console.error("Failed to sync native glass regions:", error));
  });
}

async function clearNativeGlassRegions() {
  nativeRegionGlassEnabled = false;
  if (regionSyncFrame !== 0) {
    window.cancelAnimationFrame(regionSyncFrame);
    regionSyncFrame = 0;
  }
  await invoke("set_liquid_glass_regions", { regions: [] });
}

export function installNativeGlassRegionSync() {
  if (!isTauri() || regionSyncInstalled) return;
  regionSyncInstalled = true;

  const observer = new MutationObserver(scheduleNativeGlassRegionSync);
  observer.observe(document.body, {
    attributes: true,
    childList: true,
    subtree: true,
  });

  const resizeObserver = new ResizeObserver(scheduleNativeGlassRegionSync);
  resizeObserver.observe(document.documentElement);
  resizeObserver.observe(document.body);

  window.addEventListener("resize", scheduleNativeGlassRegionSync);
  window.addEventListener("scroll", scheduleNativeGlassRegionSync, true);
  scheduleNativeGlassRegionSync();
}

export async function applyAppTheme(theme: AppTheme): Promise<ThemeResult> {
  applyThemeClass(theme);

  if (!isTauri()) {
    return { nativeGlass: false };
  }

  if (theme === "cobalt") {
    await clearNativeGlassRegions();
    await setLiquidGlassEffect({ enabled: false });
    return { nativeGlass: false };
  }

  const nativeGlass = await isGlassSupported();
  if (nativeGlass) {
    // The plugin's full-window view is disabled in favor of separate native
    // regions, grouped by NSGlassEffectContainerView for real refraction.
    await setLiquidGlassEffect({ enabled: false });
    nativeRegionGlassEnabled = true;
    scheduleNativeGlassRegionSync();
    window.setTimeout(scheduleNativeGlassRegionSync, 100);
    window.setTimeout(scheduleNativeGlassRegionSync, 300);
    return { nativeGlass: true };
  }

  await clearNativeGlassRegions();
  await setLiquidGlassEffect({
    enabled: true,
    cornerRadius: 12,
    variant: GlassMaterialVariant.Regular,
  });
  return { nativeGlass: false };
}
