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

- Always use a strong, unique `authorization` token in `config.toml`.
- Never expose the REST/WebSocket port directly to the internet without a reverse proxy.
- Use HTTPS/WSS in production (terminate TLS at your reverse proxy).
- Run the Docker container as non-root (the default image already does this).
- Regularly run `cargo audit` to check for known vulnerabilities in dependencies.
