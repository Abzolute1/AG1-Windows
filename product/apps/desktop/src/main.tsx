import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./dyad.css";
import "./app.css";
import "./redesign.css";
import "./github.css";
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
