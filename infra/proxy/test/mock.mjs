import { createServer } from 'node:http';

const role = process.argv[2];
let backendRequests = 0;
createServer(async (req, res) => {
  let body = '';
  for await (const chunk of req) body += chunk;
  if (role === 'frontend') {
    if (req.url !== '/auth/verify') {
      if (req.headers.authorization || req.headers['x-dev-user'] || req.headers['x-verify-secret']) {
        res.writeHead(400);
        return res.end('client auth headers reached frontend');
      }
      return res.end('frontend');
    }
    if (req.headers['x-verify-secret'] !== process.env.BFF_VERIFY_SECRET) {
      res.writeHead(403);
      return res.end('verify secret mismatch');
    }
    if (req.headers.authorization || req.headers['x-dev-user'] || body) {
      res.writeHead(400);
      return res.end('untrusted auth or request body reached verify');
    }
    if (req.headers.cookie === 'session=unavailable') {
      res.writeHead(503);
      return res.end('session store unavailable');
    }
    if (req.headers.cookie !== 'session=valid') {
      res.writeHead(401);
      return res.end('unauthenticated');
    }
    if (req.headers['x-forwarded-method'] !== 'GET' &&
        req.headers.origin !== `${req.headers['x-forwarded-proto']}://${req.headers['x-forwarded-host']}`) {
      res.writeHead(403);
      return res.end('CSRF rejected');
    }
    res.setHeader('Authorization', 'Bearer verified-token');
    res.setHeader('X-Verify-Method', req.headers['x-forwarded-method']);
    return res.end();
  }
  backendRequests++;
  if (req.headers.authorization !== 'Bearer verified-token' || req.headers['x-dev-user'] ||
      req.headers['x-verify-secret']) {
    res.writeHead(401);
    return res.end('invalid upstream identity');
  }
  if (req.url.startsWith('/api/shapes/')) {
    res.writeHead(200, { 'Content-Type': 'application/json', 'electric-offset': '42_0',
      'electric-handle': 'test-handle', Vary: 'Authorization' });
    res.write('[{"first":true},');
    return setTimeout(() => res.end('{"last":true}]'), 1200);
  }
  res.setHeader('Content-Type', 'application/json');
  res.end(JSON.stringify({ method: req.method, url: req.url, body,
    authorization: req.headers.authorization, devUser: req.headers['x-dev-user'] ?? null,
    backendRequests }));
}).listen(role === 'frontend' ? 3000 : Number(process.env.PORT || 8000), '0.0.0.0');
