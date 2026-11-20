import React from "react";
import ReactDOM from "react-dom/client";
import { App, MixerWindow } from "./App";
import { initApi, trackWindowState } from "./api";
import { loadLayout } from "./layout";
import "./styles.css";

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
