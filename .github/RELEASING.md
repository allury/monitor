# Release operations

## Publish a version

Use semantic versioning: compatible features increment the minor version, compatible fixes increment the patch version, and incompatible changes require an explicit compatibility review and major version increment. During the 0.x development phase, document any incompatibility in a minor release. Keep unreleased changes marked as such; changing Cargo's version does not authorize publication. Only advance `.github/release-version` when publication is approved.

Update the package version in Cargo.toml and Cargo.lock, then set `.github/release-version` to the matching `vX.Y.Z` and push to main. Alternatively, push a matching version tag. The Release workflow validates the version, runs CI, builds all four Linux artifacts, and only then creates the GitHub Release and tag at the tested commit.

All publishing and artifact uploads are performed by GitHub Actions. An existing release is never overwritten, and an existing tag must resolve to the tested commit. Publishing does not deploy or update any monitored server.

Prepare `docs/releases/<tag>.md` with the actual changes in that version and any necessary upgrade notes. Release descriptions use this versioned file, not README.md or a generated commit list. README.md contains only the project introduction, usage, and removal instructions. Keep user discussions and internal development notes out of both public introductions and release notes.

## Update an existing description

Edit `docs/releases/<tag>.md` and push to main. The Sync release descriptions workflow updates only the description of each existing release; unpublished versions are skipped. It also runs after a successful Release workflow so newly published versions receive their current approved notes. It does not change tags, binaries, checksums, or release status.
