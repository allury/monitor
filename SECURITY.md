# Security

## Supported versions

Only the latest tagged release is supported. Releases are never installed automatically.

## Deployment assumptions

- Keep the server bound to loopback and terminate TLS at a maintained reverse proxy.
- Treat the status dashboard as public information. `/admin` is protected by the single administrator secret; do not reuse that secret elsewhere.
- Administrator sessions live only in server memory and expire after 12 hours. Restart the server after a local `admin reset` so all old sessions are discarded.
- Use one token per node. Rotate a leaked token in the administration panel, or disable the node and create a replacement with a new ID. Reusing an existing ID does not rotate credentials.
- Keep `monitor.db`, the agent token file, and the service account private to the host.
- Run the supplied systemd units, or preserve equivalent filesystem, capability, namespace, and syscall restrictions.

## Data and runtime boundaries

- The agent does not write application files or keep an offline journal. Installation writes the executable, credential and service unit as a separate administrator operation.
- Filesystem restrictions include read-only `/proc` and `/sys`, inaccessible temporary directories and denied mount system calls. Host-side mount changes or a compromised kernel are outside this guarantee.
- Recorded traffic survives restarts. Unreported counters can be lost if a host reboots while disconnected; unknown traffic spanning calendar boundaries is attributed when received. Daily and monthly totals use UTC and are not billing records.
- Latency is measured every 30 seconds by updated agents. Only the latest round is retained in agent memory while disconnected. The controller keeps 30 days of history; older target configurations are not mixed into the current charts.
- Database writes are retried with a bounded queue. Graceful shutdown drains pending work, but power loss, forced termination or persistent storage failure can lose uncommitted data.
- Back up the database before upgrading. Schema migration retains existing records; handing an upgraded database to an older binary is unsupported. Agents must be updated manually to obtain new sampling behavior.

## Reporting a vulnerability

Do not publish credentials, database contents, host addresses, or exploitation details in a public issue. Contact the repository owner privately through GitHub first.
