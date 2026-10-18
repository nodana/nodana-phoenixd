# Bundle release development

The `Release node bundle` workflow runs when a `bundle-vX.Y.Z` tag is pushed.
The tag must match the Nodana version in `release.json`. Host-controller releases
are independent and live in `nodana-host`.

Before tagging, update the pinned Phoenixd archive and checksum when needed,
increment the Nodana bundle version and review the exact compatible sources in
`upgradeFrom`. An empty source list permits new installations only. Run the
verification tests described in [development.md](development.md).

Configure the GitHub environment `bundle-release` with secret
`BUNDLE_SIGNING_PRIVATE_KEY_PEM` and variable `BUNDLE_SIGNING_PUBLIC_KEY_PEM`.
Use the reviewed Ed25519 release key. The workflow builds on a native ARM runner,
smoke-tests the relocated payload, signs it outside Docker and verifies it
against the configured public key before publication. Signing does not grant
client approval for a release.

GitHub Releases contain `bundle.tar.gz`, the signed `release.json` envelope,
`release-sha256.txt` and `SHA256SUMS`. The manifest checksum identifies the
release and is distinct from the checksum of the envelope file. The manifest
binds the archive checksum and individual file checksums. GitHub build provenance
covers both the archive and envelope; consumers should verify the expected
repository, workflow and tag as well as the independently reviewed manifest
checksum and signing key.

Keep published artifacts and their source commits available. Deployment must
use these exact artifacts rather than rebuilding them. Publishing a release
does not update existing nodes. AWS publication and environment configuration
are owned by the private infrastructure repository.
