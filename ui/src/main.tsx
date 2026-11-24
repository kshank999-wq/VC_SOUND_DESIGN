import React from "react";
import ReactDOM from "react-dom/client";
import { App, MixerWindow } from "./App";
import { initApi, trackWindowState } from "./api";
import { loadLayout } from "./layout";
// Fonts are bundled: the desktop app's content policy allows no remote fonts.
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/600.css";
import "@fontsource/barlow-condensed/500.css";
import "@fontsource/barlow-condensed/600.css";
import "@fontsource/barlow-condensed/700.css";
import "./theme/tokens.css";
import "./styles.css";
import "./shell.css";

const view = new URLSearchParams(location.search).get("view");
initApi()
  .then(() => loadLayout())
  .then(() => {
    trackWindowState();
  })
  .then(() => {
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>{view === "mixer" ? <MixerWindow /> : <App />}</React.StrictMode>,
  );
});
