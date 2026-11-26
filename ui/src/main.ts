import "./style.css";
import { api, events } from "./api";
import { createApp } from "./app";

createApp(document.getElementById("app")!, { api, events });
