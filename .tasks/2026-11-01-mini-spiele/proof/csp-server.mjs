import { createServer } from 'node:http';
import { readFileSync, existsSync, statSync } from 'node:fs';
import { extname, join } from 'node:path';

const [root, caddyfile, port] = process.argv.slice(2);
const line = readFileSync(caddyfile, 'utf8').split('\n')[262];
const csp = line.match(/Content-Security-Policy "([^"]+)"/)[1];
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.json': 'application/json', '.woff2': 'font/woff2', '.png': 'image/png', '.webp': 'image/webp' };
createServer((req, res) => {
  const path = new URL(req.url, 'http://x').pathname;
  let file = join(root, path);
  if (!existsSync(file) || statSync(file).isDirectory()) file = join(root, 'index.html');
  res.writeHead(200, { 'Content-Type': types[extname(file)] ?? 'application/octet-stream', 'Content-Security-Policy': csp });
  res.end(readFileSync(file));
}).listen(Number(port), '127.0.0.1');
