# Nodana Phoenixd

Native Linux releases of Phoenixd and its Nodana launcher. Each Nodana release
has its own version and includes a pinned Phoenixd version.

Each node runs this bundle against its own persistent wallet directory. The
bundle contains Phoenixd, its runtime libraries and the small launcher in
`launcher/src/main.rs`. The launcher selects the bundled libraries and passes arguments,
signals and exit status through to Phoenixd.

To inspect the software managing nodes and producing runtime attestation, see
[nodana-host](https://github.com/nodana/nodana-host).

For local builds, release signing and verification, see the
[developer guide](docs/development.md).
