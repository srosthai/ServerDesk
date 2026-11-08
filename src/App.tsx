import { HashRouter } from "react-router-dom";
import { AppRoutes } from "@/app/AppRoutes";

function App() {
  return (
    <HashRouter>
      <AppRoutes />
    </HashRouter>
  );
}

export default App;
