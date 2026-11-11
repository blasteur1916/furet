import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { chargerLangue } from "./i18n";
import "./styles/tokens.css";

// La langue d'abord : tous les textes en dépendent.
chargerLangue().finally(() =>
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  ),
);
