# Changelog

## 0.4.0 - 2026-09-04

### Added

- Shared TCP sampling interval in administrator latency settings (10–3600 seconds, default 30), applied live by compatible agents.
- Administrator password changes with current-password verification, confirmation and immediate session revocation.
- Salted Argon2id password storage with legacy credential compatibility and bounded authentication attempts.

### Changed

- Matched Komari's bounded TCP high-latency recheck rules; failures remain stored and appear as chart gaps with optional aggregate statistics.
- Replaced unsupported expiry symbols with embedded operating-system marks, preserving monthly traffic markup.
- Aligned SQLite writer settings with Komari's main-database baseline: 8 MiB cache, 256-page checkpoint, 1 MiB retained WAL target, file-backed temporary storage and disabled mmap. Kept bounded history readers.

### Fixed

- Removed administrator credentials from startup/reset logs and addressed the same output path in CLI node creation.
- Added idempotent, terminal-only credential initialization; noninteractive credential generation/reset fails without changing a credential.
- Preserved node credentials, existing latency history and target revision when changing the sampling interval.

### Upgrade notes

- Update the controller first, then manually update agents to enable the new sampling policy and interval setting. Older agents retain their existing cadence.
- Existing administrator and node credentials remain valid. New installations and CLI password resets require an interactive terminal.

## 0.3.1 - 2026-09-04

### Changed

- Removed public-page explanatory paragraphs, repeated back navigation and sampling placeholders.
- Shortened detail system labels and latency chart titles without changing recorded metrics or failure indicators.
- Kept site descriptions as page metadata and preserved administrator settings and optional footers.

## 0.3.0 - 2026-09-04

### Added

- Manual agent update mode that preserves installed credentials, endpoint and service configuration.
- Administrator node detail and installation/update dialog with explicit hash-only credential status.
- Bounded live network sparklines and chart cursors with click/touch pinning and keyboard selection.
- In-page navigation between the status page, node details and administrator view.

### Changed

- Aligned navigation, overview and four-column node cards with the reference layout.
- Shortened system labels and standardized resource/transfer units; preserved monthly traffic markup and binary units.
- Made the default footer empty while retaining the optional setting.
- Simplified chart tooltips, showing failure counts only when failures occur.
- Removed the page-level update timestamp while retaining node freshness indicators.

### Fixed

- Kept update commands available when a credential is unavailable or a node detail request fails.
- Added atomic binary replacement and rollback when the updated agent cannot restart.
- Verified credential retention across database migration and controller restart.

## 0.2.0 - 2026-09-04

### Added

- Node detail pages with resource and TCP latency history.
- Bounded history queries with server-side aggregation and 30-day retention.
- Explicit node credential rotation with replacement installation commands.
- Integration coverage for reporting, connection lifecycle, persistence and service sandboxing.

### Changed

- Refined the status page with compact, responsive node cards and separate upload/download values.
- Decoupled TCP probes from status collection, using a 30-second probe interval.
- Stored each latency round independently and separated history by target revision.
- Tracked network counter deltas per interface and filtered common virtual interfaces.

### Fixed

- Invalidated live and queued reports from revoked or replaced node credentials.
- Preserved pending database batches for retry and drained queues during graceful shutdown.
- Corrected daily and monthly totals for offline nodes after calendar rollover.
- Marked stale browser snapshots explicitly and avoided resetting focused node links.
- Added HTTP clipboard fallback, shared theme initialization and static asset revalidation.
- Restricted agent access to temporary directories and corrected unknown virtualization reporting.

### Upgrade notes

- Existing node state and resource history are retained by the database migration.
- Upgrade the controller first, then manually upgrade agents for 30-second latency collection.
- Agent updates remain manual. No remote execution, notifications or plugin system is included.
