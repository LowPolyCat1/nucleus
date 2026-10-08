// Egress proxy for nucleus sandboxes.
//
// Runs in a sidecar container that sits on both the sandbox's internal network and a network
// with internet access. The sandbox reaches the outside world only through this proxy, which
// allows CONNECT tunnels and plain HTTP requests to hosts on the allowlist and rejects the rest.
//
// NUCLEUS_ALLOW: comma separated hosts. `example.com` matches exactly, `*.example.com` matches
// subdomains only, `*` matches everything.
'use strict';
const http = require('http');
const net = require('net');

const PORT = Number(process.env.NUCLEUS_EGRESS_PORT || 3128);
const ALLOW = (process.env.NUCLEUS_ALLOW || '')
  .split(',')
  .map((h) => h.trim().toLowerCase())
  .filter(Boolean);

function allowed(host) {
  host = String(host || '').toLowerCase().replace(/\.$/, '');
  return ALLOW.some((rule) => {
    if (rule === '*') return true;
    if (rule.startsWith('*.')) return host.endsWith(rule.slice(1));
    return host === rule;
  });
}

function log(verdict, target) {
  process.stdout.write(JSON.stringify({ t: Date.now(), verdict, target }) + '\n');
}

const server = http.createServer((req, res) => {
  let url;
  try {
    url = new URL(req.url);
  } catch {
    res.writeHead(400).end('absolute URI required\n');
    return;
  }
  if (!allowed(url.hostname)) {
    log('deny', url.host);
    res.writeHead(403).end(`nucleus: egress to ${url.hostname} is not allowed\n`);
    return;
  }
  log('allow', url.host);
  const upstream = http.request(
    { host: url.hostname, port: url.port || 80, method: req.method, path: url.pathname + url.search, headers: req.headers },
    (up) => {
      res.writeHead(up.statusCode, up.headers);
      up.pipe(res);
    },
  );
  upstream.on('error', () => res.destroy());
  req.pipe(upstream);
});

server.on('connect', (req, client, head) => {
  const [host, portStr] = req.url.split(/:(?=\d+$)/);
  const port = Number(portStr || 443);
  if (!allowed(host)) {
    log('deny', req.url);
    client.end(`HTTP/1.1 403 Forbidden\r\n\r\nnucleus: egress to ${host} is not allowed\n`);
    return;
  }
  log('allow', req.url);
  const upstream = net.connect(port, host, () => {
    client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
    if (head && head.length) upstream.write(head);
    upstream.pipe(client);
    client.pipe(upstream);
  });
  upstream.on('error', () => client.destroy());
  client.on('error', () => upstream.destroy());
});

server.listen(PORT, '0.0.0.0', () => log('listening', String(PORT)));
