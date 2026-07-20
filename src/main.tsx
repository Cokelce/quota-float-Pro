import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { DesignPlayground } from "./components/DesignPlayground";
import { SettingsApp } from "./SettingsApp";
import { StatusBarApp } from "./StatusBarApp";
import "./styles.css";

const params = new URLSearchParams(window.location.search);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{params.has("designer") ? <DesignPlayground /> : params.has("settings") ? <SettingsApp /> : params.has("statusbar") ? <StatusBarApp /> : <App />}</React.StrictMode>,
);
