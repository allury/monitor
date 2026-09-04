# Changelog

## 0.3.0 - Unreleased

### Added

- Manual agent update mode that preserves installed credentials, endpoint and service configuration.
- Administrator node detail and installation/update dialog with explicit hash-only credential status.
- Bounded live network sparklines and chart cursors with click/touch pinning and keyboard selection.
- In-page navigation between the status page, node details and administrator view.

### Changed

- Aligned navigation, overview and four-column node cards with the reference layout.
- Shortened system labels and standardized resource/transfer units; preserved monthly traffic markup and binary units.
- Made the default footer empty while retaining the optional setting.

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
