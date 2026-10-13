import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../styles/app.css";
import "./overlay.css";
import { OverlayApp } from "./OverlayApp";

const rootEl = document.getElementById("root");
if (rootEl)
  createRoot(rootEl).render(
    <StrictMode>
      <OverlayApp />
    </StrictMode>,
  );
