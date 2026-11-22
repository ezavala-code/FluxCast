import { createRoot } from "react-dom/client";
import { CommandBar } from "./components/CommandBar";
import "./style.css";

createRoot(document.getElementById("root")!).render(<CommandBar />);
