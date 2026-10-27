import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { disableNativeContextMenu } from "./contextMenu";
import "./index.css";
import { guardTextFields } from "./textInput";

const root = document.getElementById("root");
if (!root) {
  throw new Error("root element missing");
}

disableNativeContextMenu(document, import.meta.env.PROD);
guardTextFields(document);

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
