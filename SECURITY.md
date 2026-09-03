# Security

## Supported versions

Only the latest tagged release is supported. Releases are never installed automatically.

## Deployment assumptions

- Keep the server bound to loopback and terminate TLS at a maintained reverse proxy.
- Treat the status dashboard as public information. `/admin` is protected by the single administrator secret; do not reuse that secret elsewhere.
- Administrator sessions live only in server memory and expire after 12 hours. Restart the server after a local `admin reset` so all old sessions are discarded.
- Use one token per node. Revoke and recreate the node token if it may have leaked.
- Keep `monitor.db`, the agent token file, and the service account private to the host.
- Run the supplied systemd units, or preserve equivalent filesystem, capability, namespace, and syscall restrictions.

## Reporting a vulnerability

Do not publish credentials, database contents, host addresses, or exploitation details in a public issue. Contact the repository owner privately through GitHub first.
