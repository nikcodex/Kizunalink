# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.x     | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability in KizunaLink, please report it responsibly:

1. **Do NOT** open a public GitHub issue.
2. Email security concerns to the maintainers or use GitHub's private vulnerability reporting.
3. Include:
   - A description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

We aim to acknowledge reports within 48 hours and provide a fix or mitigation plan within 7 days for critical issues.

## Security Best Practices for Deployment

- Always use a strong, unique `authorization` token in `config.toml` or via
  `KIZUNA_AUTHORIZATION`. There is no default: the server refuses to start if the
  secret is missing, empty, or a known placeholder (including `youshallnotpass`),
  on every bind address including loopback.
- Never expose the REST/WebSocket port directly to the internet without a reverse proxy.
- A reverse proxy is not a substitute for the secret. Bind the backend to a
  private address (`127.0.0.1` or a container network) so the port is not
  directly reachable, and always set a strong `authorization` value. If the proxy
  manages auth, it must preserve or inject the `Authorization` header end-to-end.
- Use HTTPS/WSS in production (terminate TLS at your reverse proxy, or enable
  `server.tls`).
- `/health` is unauthenticated for orchestrators; do not expose it publicly.
- Run the Docker container as non-root (the default image already does this).
- Regularly run `cargo audit` to check for known vulnerabilities in dependencies.
