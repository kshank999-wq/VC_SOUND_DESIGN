import React from "react";
import ReactDOM from "react-dom/client";
import { App, MixerWindow } from "./App";
import { initApi } from "./api";
import "./styles.css";

const view = new URLSearchParams(location.search).get("view");
initApi().then(() => {
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>{view === "mixer" ? <MixerWindow /> : <App />}</React.StrictMode>,
  );
});
