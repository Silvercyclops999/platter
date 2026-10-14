import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./design/global.css";
import "./components/components.css";
import "./shell/shell.css";
import "./pages/pages.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
