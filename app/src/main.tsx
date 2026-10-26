import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { disableNativeContextMenu } from "./contextMenu";
import "./index.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("root element missing");
}

disableNativeContextMenu(document, import.meta.env.PROD);

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
