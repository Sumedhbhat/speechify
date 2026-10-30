import './style.css';
import { startApplication } from './app';

const app = document.querySelector<HTMLDivElement>('#app')!;
const overlay = new URLSearchParams(location.search).has('overlay');
startApplication(app, overlay);
