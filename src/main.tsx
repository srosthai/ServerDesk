import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";

const storedTheme = localStorage.getItem("serverdesk-theme");
const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
const useDark = storedTheme === "dark" || (storedTheme !== "light" && prefersDark);

document.documentElement.classList.toggle("dark", useDark);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
