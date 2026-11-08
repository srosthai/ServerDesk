import { HashRouter } from "react-router-dom";
import { AppRoutes } from "@/app/AppRoutes";
import { ThemeSync } from "@/app/ThemeSync";

function App() {
  return (
    <>
      <ThemeSync />
      <HashRouter>
        <AppRoutes />
      </HashRouter>
    </>
  );
}

export default App;
