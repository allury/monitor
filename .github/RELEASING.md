# Release operations

## Publish a version

Update the package version in Cargo.toml and Cargo.lock, then set `.github/release-version` to the matching `vX.Y.Z` and push to main. Alternatively, push a matching version tag. The Release workflow validates the version, runs CI, builds all four Linux artifacts, and only then creates the GitHub Release and tag at the tested commit.

All publishing and artifact uploads are performed by GitHub Actions. An existing release is never overwritten, and an existing tag must resolve to the tested commit. Publishing does not deploy or update any monitored server.

Release descriptions come from README.md and contain only the project introduction, usage, and removal instructions. Keep internal development notes in CHANGELOG.md or dedicated engineering documents, not in release descriptions.

## Update an existing description

Edit `docs/releases/<tag>.md` and push to main. The Sync release descriptions workflow updates only the description of each existing release. It does not change tags, binaries, checksums, or release status.
