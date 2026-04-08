# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do NOT open a public issue**
2. Email: security@alpibrusl.dev (or open a private security advisory on GitHub)
3. Include: description, steps to reproduce, potential impact

We will acknowledge receipt within 48 hours and provide a timeline for a fix.

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Security Considerations

- Agent credentials are injected via temporary files, not environment variables
- The Noether KV store uses local SQLite — not exposed to the network
- GitHub tokens should use minimum required scopes (`repo`, `workflow`)
- The shell HTTP server binds to `127.0.0.1` only (not exposed externally)
