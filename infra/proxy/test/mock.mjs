import { createServer } from 'node:http';

const role = process.argv[2];
createServer(async (req, res) => {
  let body = '';
  for await (const chunk of req) body += chunk;
  if (role === 'frontend') {
    return res.end('frontend');
  }
  if (req.headers['x-dev-user']) {
    res.writeHead(400);
    return res.end('untrusted X-Dev-User header reached backend');
  }
  if (req.headers.authorization !== 'Bearer valid-token') {
    res.writeHead(401);
    return res.end('unauthenticated');
  }
  if (req.url.startsWith('/api/shapes/')) {
    res.writeHead(200, { 'Content-Type': 'application/json', 'electric-offset': '42_0',
      'electric-handle': 'test-handle', Vary: 'Authorization' });
    res.write('[{"first":true},');
    return setTimeout(() => res.end('{"last":true}]'), 1200);
  }
  res.setHeader('Content-Type', 'application/json');
  res.end(JSON.stringify({ method: req.method, url: req.url, body,
    authorization: req.headers.authorization, devUser: req.headers['x-dev-user'] ?? null }));
}).listen(role === 'frontend' ? 3000 : Number(process.env.PORT || 8000), '0.0.0.0');

