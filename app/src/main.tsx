import { render } from "@solidjs/web";
import { createBackend } from "./api";
import { App } from "./components/App";
import { createApp } from "./store";
import "./styles.css";

const app = createApp(createBackend());
render(() => <App app={app} />, document.getElementById("root")!);
