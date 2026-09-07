import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { LogWindow } from "./components/LogWindow";
import { FileExplorer } from "./components/FileExplorer";
import { applyScale, getSettings } from "./components/Settings";
import { applyAppTheme, applyThemeClass, installNativeGlassRegionSync } from "./theme";
import "remixicon/fonts/remixicon.css";
import "./App.css";

const initialSettings = getSettings();
const isAuxiliaryWindow = /^#\/(logs|files)\//.test(window.location.hash);

// Auxiliary windows stay on the opaque utility theme for log/file readability.
applyScale(initialSettings.uiScale);
applyThemeClass(isAuxiliaryWindow ? "cobalt" : initialSettings.theme);
if (!isAuxiliaryWindow) {
  void applyAppTheme(initialSettings.theme).catch((error) => {
    console.error("Failed to restore app theme:", error);
    applyThemeClass("cobalt");
  });
}

function Router() {
  const hash = window.location.hash;
  const logMatch = hash.match(/^#\/logs\/([^/]+)\/(.+)$/);
  const fileMatch = hash.match(/^#\/files\/([^/]+)\/(.+)$/);

  if (logMatch) {
    return <LogWindow containerId={logMatch[1]} containerName={decodeURIComponent(logMatch[2])} />;
  }
  if (fileMatch) {
    return <FileExplorer containerId={fileMatch[1]} containerName={decodeURIComponent(fileMatch[2])} />;
  }

  return <App />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Router />
  </React.StrictMode>,
);

if (!isAuxiliaryWindow) {
  installNativeGlassRegionSync();
}
