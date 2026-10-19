import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./design/global.css";
import { applyCachedTheme } from "./design/themes";
import { applyCachedLook } from "./design/looks/apply";
import "./components/components.css";
import "./shell/shell.css";
import "./pages/pages.css";
import "./design/looks/looks.css";
import "./design/identity.css";
import "./design/instruments.css";

applyCachedTheme();
applyCachedLook();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
