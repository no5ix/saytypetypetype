import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../styles/app.css";
import "./picker.css";
import { PickerApp } from "./PickerApp";

const rootEl = document.getElementById("root");
if (rootEl)
  createRoot(rootEl).render(
    <StrictMode>
      <PickerApp />
    </StrictMode>,
  );
