import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AppShell } from "@/app/app-shell";
import "./index.css";

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("Root element not found");
}

createRoot(rootElement).render(
  <StrictMode>
    <AppShell />
  </StrictMode>
);
