import { attach } from './page.mjs';
attach(document, location.href, history, url => location.assign(url));
