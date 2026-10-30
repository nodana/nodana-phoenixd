# Native release development

For GitHub builds and release signing configuration, see
[bundle releases](releasing.md).

`release.json` versions the Nodana bundle independently of `phoenixdVersion`.
`phoenixd.nix` pins the upstream archive for ARM64. `build.nix` pins the Nix
package set. Change both the metadata and upstream pin when changing Phoenixd.
The local builder currently targets native ARM Docker Desktop.

## Build locally

From this directory, with Docker Desktop running on an ARM Mac:

```sh
cargo install --path tools --locked
ndn-bundle build --output /private/tmp/nodana-arm-build
```

Choose a fresh output directory. The builder copies only the allowlisted Nix files,
Rust launcher sources and `release.json` into an unmounted disposable container, with a 6 GiB memory
and 15 GiB Nix-store budget. It creates no EC2 resources or host images.
It copies the resulting payload out, then checks both versions after relocation
inside a clean Alpine ARM container with networking disabled. Both containers
and newly pulled images are removed after success or failure. Nix caches live
only in the builder container. `build.log` and `build-result.json` retain evidence.

The statically linked launcher uses the payload's glibc loader and resolved ELF
dependencies. It replaces itself with Phoenixd so signals and exit codes pass
through. The host controller supplies mainnet flags, per-node ports and the
persistent wallet directory. `--nodana-version` prints the bundle version;
`--version` prints the included Phoenixd version without starting a wallet.

## Temporary CA compatibility

This is a temporary bridge for existing hosts whose system trust store uses
OpenSSL `TRUSTED CERTIFICATE` blocks or lacks `/etc/ssl/cert.pem`. It allows a
per-node runtime update without changing or restarting the shared host. Future
host images carry the system CA fix independently.

The Linux launcher embeds the standard PEM CA bundle from the pinned Nix package
and sets `SSL_CERT_FILE` to a sealed anonymous file retained across exec. It clears
inherited CA directory and curl-bundle overrides. This provides runtime-owned
trust roots without writing to the host or wallet filesystem; updating those
roots requires a new bundle release. The certificate bytes are covered by the
signed launcher's checksum, preserving the existing ELF-only bundle contract.
Release builds require `NODANA_CA_BUNDLE` at compile time. For standalone Linux
launcher development, set it to a standard PEM CA bundle before `cargo build`.

Remove this workaround only after every affected host has been fixed or retired,
and Phoenixd HTTPS has been verified using the system trust store with no
`SSL_CERT_FILE` override. Completing a trial on one node is not sufficient.
In particular, an empty `swift` can stop requiring this bridge once it has been
retired from new placements. Nodes on a second EC2 built with the corrected host
image do not depend on this bridge. Do not remove it while any node still runs
on an affected host, or while that host can receive new placements.

For removal, delete `launcher/src/trust.rs` and `launcher/build.rs`, remove the
launcher setup of the embedded CA file and CA environment overrides, remove the
Linux `libc` dependency and regenerate its lockfile, remove `NODANA_CA_BUNDLE`
from `bundle.nix`, and remove the deleted files from `tools/src/build.rs`'s source
allowlist. Keep loader hardening, argument forwarding, wallet recovery and
release verification coverage. Build and test a new release; do not alter
already published bundles or their manifests. Keep this section until removal
has been verified on a representative host.

## Sign and verify

Install Rust and OpenSSL 3, including its development libraries. On macOS, put
Homebrew's OpenSSL 3 `bin` directory on `PATH` for the commands below.
Keep signing keys outside this repository
and outside Docker. For a disposable local verification key:

```sh
umask 077
openssl genpkey -algorithm ED25519 -out /private/tmp/nodana-test-signing.pem
openssl pkey -in /private/tmp/nodana-test-signing.pem -pubout -out /private/tmp/nodana-test-signing.pub
ndn-bundle pack --payload /private/tmp/nodana-arm-build/payload \
  --metadata release.json --private-key /private/tmp/nodana-test-signing.pem \
  --public-key /private/tmp/nodana-test-signing.pub \
  --output /private/tmp/nodana-test-release
```

`pack` prints the exact release approval checksum. Supply that checksum explicitly:

```sh
ndn-bundle verify --release /private/tmp/nodana-test-release \
  --public-key /private/tmp/nodana-test-signing.pub --approve-sha256 REVIEWED_CHECKSUM
```

Do not put disposable keys or their checksums into shipped trust policies.
Release signing trust and checksum approval are separate inputs. Neither is
obtained from an API response or from the release itself.

## Bundle contract

A release consists of `release.json` and `bundle.tar.gz`. The envelope contains
`manifest`, `keyId` (SHA256 of the raw Ed25519 public key), and a lowercase hex
Ed25519 signature over `nodana-native-release-v1\0` followed by the canonical
manifest. Canonical encoding is sorted-key ASCII JSON without whitespace.
Manifest strings are constrained to ASCII. Duplicate or unknown fields fail
verification. The approval checksum is SHA256 of that canonical manifest.

The manifest binds Nodana and Phoenixd versions, architecture, state format,
explicit update sources, archive checksum and size, and every payload file's
path, checksum, size and mode. Archives are deterministic gzip/USTAR, with regular
files only, zero ownership/timestamps, bounded sizes, and no symlinks, hard links,
path traversal or extra members. Every ELF must match the selected architecture.

`upgradeFrom` lists exact `{version, releaseSha256, stateFormat}` source identities.
An update must advance the Nodana version and keep the architecture. A signer
must explicitly review state compatibility for every source. An empty list makes
the release eligible only for initial installation. Failed updates must retain
the target selection and wallet directory for repair; do not automatically boot
an older binary against potentially migrated channel databases.

The Rust controller in `nodana-host` verifies everything before creating a
content-addressed, read-only payload directory. It rechecks the trusted signature,
approval checksum, ownership, permissions and actual executable bytes during use.
Activation and node lifecycle operations belong in `nodana-host`.
Never install executable files beneath a wallet-owned directory.

Environment publication consumes the public release artifacts separately from
these builds; it requires no private repository access from this repository.

## Verify changes

```sh
cargo fmt --manifest-path tools/Cargo.toml -- --check
cargo test --manifest-path tools/Cargo.toml --locked
cargo test --manifest-path launcher/Cargo.toml --locked
git diff --check
```

Tests generate temporary keys and synthetic ELF fixtures, exercise signature and
approval failures and archive validation. The public host repository tests
update compatibility, installed-byte verification and isolated staging.
The Docker relocation test checks actual ARM executables. It does not start a
funded wallet or prove host update recovery.

Recorded ARM build evidence: [2026-10-23 results](local-arm-results-2026-10-23.json).
