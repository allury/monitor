# Security

## Supported versions

Only the latest tagged release is supported. Releases are never installed automatically.

## Deployment assumptions

- Keep the server bound to loopback and terminate TLS at a maintained reverse proxy.
- Treat the status dashboard as public information. `/admin` uses a single administrator password; do not reuse it elsewhere. HTTP is for temporary testing only and does not protect passwords, tokens or metrics in transit.
- New administrator credentials use salted Argon2id (19 MiB, two passes, one lane). Existing random credentials with SHA-256 digests remain valid; a password change replaces their digest with Argon2id.
- Password changes require the current password and revoke all administrator sessions immediately. Sessions live only in server memory, expire after 12 hours, and use a bearer token stored in the browser tab's session storage. Restart the server after a local `admin reset` to activate that reset and discard old sessions.
- Login and password changes share a five-attempt, 60-second administrator-wide budget and a single password-verification slot. Proxy headers cannot bypass this limit. This bounds hashing work but is not a defense against all denial-of-service attacks.
- The daemon never generates or prints credentials. First-time `admin init`, `admin reset` and CLI node creation require a controlling terminal and deliver their secret directly there, not to stdout/stderr or service logs. Terminal recording software can still capture it; keep terminal sessions private. Repeating `admin init` preserves an existing credential.
- Use one token per node. Rotate a leaked token in the administration panel, or disable the node and create a replacement with a new ID. Reusing an existing ID does not rotate credentials.
- Keep `monitor.db`, the agent token file, and the service account private to the host.
- Run the supplied systemd units, or preserve equivalent filesystem, capability, namespace, and syscall restrictions.

## Data and runtime boundaries

- The agent does not write application files or keep an offline journal. Installation writes the executable, credential and service unit as a separate administrator operation.
- Filesystem restrictions include read-only `/proc` and `/sys`, inaccessible temporary directories and denied mount system calls. Host-side mount changes or a compromised kernel are outside this guarantee.
- Recorded traffic survives restarts. Unreported counters can be lost if a host reboots while disconnected; unknown traffic spanning calendar boundaries is attributed when received. Daily and monthly totals use UTC and are not billing records.
- Latency defaults to 30-second sampling; compatible agents accept a shared 10–3600 second interval from the administrator. Komari-style TCP high-latency rechecks are bounded to three additional attempts; failed samples remain recorded and are shown as gaps with optional aggregate statistics. Only the latest round is retained in agent memory while disconnected. The controller keeps 30 days of history; older target configurations are not mixed into the current charts.
- Database writes are retried with a bounded queue. Graceful shutdown drains pending work, but power loss, forced termination or persistent storage failure can lose uncommitted data.
- Back up the database before upgrading. Schema migration retains existing records; handing an upgraded database to an older binary is unsupported. Agents must be updated manually to obtain new sampling behavior.

## Reporting a vulnerability

Do not publish credentials, database contents, host addresses, or exploitation details in a public issue. Contact the repository owner privately through GitHub first.
