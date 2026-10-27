# 1Panel / Nginx reverse proxy

1. Deploy DSH Docker with the default loopback binding:

   ```bash
   dshctl deploy
   ```

2. In 1Panel create a website for your domain and enable HTTPS.

3. Add a reverse proxy to:

   ```text
   http://127.0.0.1:3080
   ```

4. Set the proxy options:

   - Preserve the original `Host` and `Origin` headers. Do not rewrite them to
     `localhost`.
   - Enable WebSocket support (`Upgrade` / `Connection`).
   - Disable response buffering for SSE.
   - Increase read timeout for long agent sessions.

5. Do not add another Basic Auth layer unless you intend to maintain two credential
   stores. The DSH gateway already authenticates and caches bcrypt credentials.

6. Open the domain in a browser. An unauthenticated request should return `401`;
   after login the DSH UI should load and `/__dsh_http_compat.js` should be injected
   into the HTML (visible in devtools).
