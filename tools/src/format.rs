//! Signed native bundles. Wallet paths and executable selection are never inputs.
use crate::wire::{canonical, digest, parse_json};
use anyhow::{Context, Result, ensure};
use flate2::read::MultiGzDecoder;
use openssl::{
    pkey::{Id, PKey, Public},
    sign::Verifier,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;
const MAX_PAYLOAD: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = MAX_PAYLOAD + 256 * 1024 + 10240;
const DOMAIN: &[u8] = b"nodana-native-release-v1\0";

fn expand(compressed: &[u8], maximum: u64) -> Result<Vec<u8>> {
    let mut expanded = Vec::new();
    MultiGzDecoder::new(compressed)
        .take(maximum + 1)
        .read_to_end(&mut expanded)?;
    ensure!(
        expanded.len() as u64 <= maximum,
        "expanded archive too large"
    );
    Ok(expanded)
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    version: String,
    release_sha256: String,
    state_format: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    path: String,
    sha256: String,
    size: u64,
    mode: u32,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    schema_version: u8,
    version: String,
    phoenixd_version: String,
    platform: String,
    state_format: String,
    upgrade_from: Vec<Source>,
    entrypoint: String,
    bundle_sha256: String,
    bundle_size: u64,
    files: Vec<Payload>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    manifest: Value,
    key_id: String,
    signature: String,
}

pub fn checksum(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn hash(data: &[u8]) -> String {
    hex::encode(digest(data))
}
fn version(s: &str) -> Result<semver::Version> {
    ensure!(
        regex::Regex::new(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")?.is_match(s),
        "invalid version"
    );
    Ok(semver::Version::parse(s)?)
}
fn state(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-".contains(&b))
}
fn elf(data: &[u8]) -> Result<()> {
    ensure!(
        data.len() >= 64 && &data[..6] == b"\x7fELF\x02\x01" && data[18..20] == [183, 0],
        "invalid ARM64 ELF"
    );
    Ok(())
}
pub fn read(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= maximum,
        "invalid file"
    );
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= maximum, "oversized file");
    Ok(bytes)
}
#[cfg(test)]
fn mode(path: &Path, value: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(value))?;
    Ok(())
}
fn validate(value: &Value) -> Result<Manifest> {
    let m: Manifest = serde_json::from_value(value.clone())?;
    let target = version(&m.version)?;
    version(&m.phoenixd_version)?;
    ensure!(
        m.schema_version == 1
            && m.platform == "linux-arm64"
            && state(&m.state_format)
            && m.entrypoint == "bin/nodana-phoenixd",
        "invalid manifest"
    );
    ensure!(
        checksum(&m.bundle_sha256)
            && (1..=MAX_ARCHIVE).contains(&m.bundle_size)
            && m.upgrade_from.len() <= 128,
        "invalid bundle"
    );
    let mut sources = BTreeSet::new();
    for s in &m.upgrade_from {
        ensure!(
            version(&s.version)? < target
                && checksum(&s.release_sha256)
                && state(&s.state_format)
                && sources.insert(&s.release_sha256),
            "invalid update path"
        );
    }
    ensure!((3..=256).contains(&m.files.len()), "invalid payload count");
    let pattern = regex::Regex::new(r"^(bin|libexec|lib)/[a-zA-Z0-9_+.-]{1,128}$")?;
    let mut names = BTreeSet::new();
    let mut total = 0;
    for item in &m.files {
        ensure!(
            pattern.is_match(&item.path)
                && !matches!(item.path.split('/').nth(1), Some("." | ".."))
                && names.insert(item.path.as_str()),
            "invalid payload path"
        );
        ensure!(
            checksum(&item.sha256)
                && (1..=MAX_PAYLOAD).contains(&item.size)
                && matches!(item.mode, 0o444 | 0o555),
            "invalid payload"
        );
        total += item.size;
        ensure!(total <= MAX_PAYLOAD, "oversized payload");
    }
    for required in ["bin/nodana-phoenixd", "libexec/phoenixd", "lib/ld.so"] {
        ensure!(
            m.files
                .iter()
                .any(|f| f.path == required && f.mode == 0o555),
            "missing executable"
        );
    }
    Ok(m)
}
fn envelope(
    directory: &Path,
    key: &PKey<Public>,
    approval: &str,
) -> Result<(Manifest, Value, Vec<u8>)> {
    ensure!(checksum(approval), "explicit checksum required");
    let bytes = read(&directory.join("release.json"), 256 * 1024)?;
    let e: Envelope = serde_json::from_value(parse_json(&bytes)?)?;
    let manifest = validate(&e.manifest)?;
    let encoded = canonical(&e.manifest);
    ensure!(
        hash(&encoded) == approval
            && key.id() == Id::ED25519
            && hash(&key.raw_public_key()?) == e.key_id,
        "unapproved release or signing key"
    );
    ensure!(
        e.signature.len() == 128
            && e.signature
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid signature"
    );
    let mut message = DOMAIN.to_vec();
    message.extend(encoded);
    ensure!(
        Verifier::new_without_digest(key)?.verify_oneshot(&hex::decode(&e.signature)?, &message)?,
        "invalid release signature"
    );
    Ok((manifest, e.manifest, bytes))
}
fn archive(directory: &Path, m: &Manifest) -> Result<BTreeMap<String, Vec<u8>>> {
    let compressed = read(&directory.join("bundle.tar.gz"), MAX_ARCHIVE)?;
    ensure!(
        compressed.len() as u64 == m.bundle_size && hash(&compressed) == m.bundle_sha256,
        "archive checksum mismatch"
    );
    let expanded = expand(&compressed, MAX_EXPANDED)?;
    let mut tar = tar::Archive::new(expanded.as_slice());
    let mut content = BTreeMap::new();
    let mut end = 0;
    for entry in tar.entries()?.raw(true) {
        let mut entry = entry?;
        let name = String::from_utf8(entry.path_bytes().to_vec())?;
        let item = m
            .files
            .iter()
            .find(|f| f.path == name)
            .context("unexpected archive member")?;
        let header = entry.header();
        ensure!(
            !content.contains_key(&name)
                && header.entry_type().is_file()
                && header.mode()? == item.mode
                && header.size()? == item.size,
            "invalid archive member"
        );
        ensure!(
            header.uid()? == 0 && header.gid()? == 0 && header.mtime()? == 0,
            "noncanonical archive metadata"
        );
        end = entry.raw_file_position() + item.size.div_ceil(512) * 512;
        let mut data = Vec::new();
        entry.by_ref().take(item.size + 1).read_to_end(&mut data)?;
        ensure!(
            data.len() as u64 == item.size && hash(&data) == item.sha256,
            "payload checksum mismatch"
        );
        elf(&data)?;
        content.insert(name, data);
    }
    ensure!(
        content.len() == m.files.len()
            && end <= expanded.len() as u64
            && expanded.len() as u64 - end <= 10240
            && expanded[end as usize..].iter().all(|b| *b == 0),
        "incomplete archive or trailing data"
    );
    Ok(content)
}

pub fn read_key(path: &Path) -> Result<Vec<u8>> {
    read(path, 16384)
}
pub fn verify(directory: &Path, public: &[u8], approval: &str) -> Result<Value> {
    let key = PKey::public_key_from_pem(public)?;
    let (manifest, value, _) = envelope(directory, &key, approval)?;
    archive(directory, &manifest)?;
    Ok(json!({"releaseSha256":approval,"manifest":value}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Metadata {
    version: String,
    phoenixd_version: String,
    state_format: String,
    upgrade_from: Vec<Value>,
}
pub fn pack(
    payload: &Path,
    metadata: &Path,
    private: &[u8],
    public: &[u8],
    output: &Path,
) -> Result<String> {
    use openssl::sign::Signer;
    ensure!(
        output.symlink_metadata().is_err(),
        "choose a new release directory"
    );
    let meta: Metadata = serde_json::from_value(parse_json(&read(metadata, 256 * 1024)?)?)?;
    let key = PKey::private_key_from_pem(private)?;
    let trusted = PKey::public_key_from_pem(public)?;
    ensure!(
        key.id() == Id::ED25519
            && trusted.id() == Id::ED25519
            && key.raw_public_key()? == trusted.raw_public_key()?,
        "signing key does not match approved public key"
    );
    let mut files = Vec::new();
    let mut content = BTreeMap::new();
    let mut total = 0;
    let name = regex::Regex::new(r"^[a-zA-Z0-9_+.-]{1,128}$")?;
    for directory in fs::read_dir(payload)? {
        let directory = directory?;
        let part = directory.file_name().to_string_lossy().into_owned();
        ensure!(
            ["bin", "lib", "libexec"].contains(&part.as_str()) && directory.file_type()?.is_dir(),
            "unexpected payload directory"
        );
        for file in fs::read_dir(directory.path())? {
            let file = file?;
            let basename = file
                .file_name()
                .to_str()
                .context("invalid filename")?
                .to_owned();
            ensure!(
                name.is_match(&basename) && basename != "." && basename != "..",
                "invalid filename"
            );
            let data = read(&file.path(), MAX_PAYLOAD)?;
            elf(&data)?;
            total += data.len() as u64;
            ensure!(
                total <= MAX_PAYLOAD && files.len() < 256,
                "payload too large"
            );
            let mode = if file.metadata()?.mode() & 0o111 != 0 {
                0o555
            } else {
                0o444
            };
            let path = format!("{part}/{basename}");
            files.push(json!({"path":path,"sha256":hash(&data),"size":data.len(),"mode":mode}));
            content.insert(path, data);
        }
    }
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let encoder = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for item in &files {
        let path = item["path"].as_str().unwrap();
        let data = &content[path];
        let mut header = tar::Header::new_ustar();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_mode(item["mode"].as_u64().unwrap() as u32);
        header.set_size(data.len() as u64);
        header.set_cksum();
        builder.append_data(&mut header, path, data.as_slice())?;
    }
    let bundle = builder.into_inner()?.finish()?;
    ensure!(bundle.len() as u64 <= MAX_ARCHIVE, "archive too large");
    let manifest = json!({"schemaVersion":1,"version":meta.version,"phoenixdVersion":meta.phoenixd_version,"stateFormat":meta.state_format,"upgradeFrom":meta.upgrade_from,"platform":"linux-arm64","entrypoint":"bin/nodana-phoenixd","bundleSha256":hash(&bundle),"bundleSize":bundle.len(),"files":files});
    validate(&manifest)?;
    let encoded = canonical(&manifest);
    let sha = hash(&encoded);
    let mut signed = DOMAIN.to_vec();
    signed.extend(encoded);
    let envelope = json!({"manifest":manifest,"keyId":hash(&key.raw_public_key()?),"signature":hex::encode(Signer::new_without_digest(&key)?.sign_oneshot_to_vec(&signed)?)});
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = tempfile::tempdir_in(parent)?;
    let stage = temporary.path().join("release");
    fs::create_dir(&stage)?;
    fs::write(stage.join("bundle.tar.gz"), bundle)?;
    let mut envelope = canonical(&envelope);
    envelope.push(b'\n');
    fs::write(stage.join("release.json"), envelope)?;
    verify(&stage, public, &sha)?;
    fs::write(stage.join("release-sha256.txt"), format!("{sha}\n"))?;
    let mut sums = String::new();
    for name in ["bundle.tar.gz", "release.json", "release-sha256.txt"] {
        sums.push_str(&format!("{}  {name}\n", hash(&fs::read(stage.join(name))?)));
    }
    fs::write(stage.join("SHA256SUMS"), sums)?;
    fs::rename(stage, output)?;
    Ok(sha)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Vec<u8>, Vec<u8>, String) {
        let root = tempfile::tempdir().unwrap();
        let mut elf = vec![0; 64];
        elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
        elf[18] = 183;
        for name in ["bin/nodana-phoenixd", "libexec/phoenixd", "lib/ld.so"] {
            let path = root.path().join("payload").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &elf).unwrap();
            mode(&path, 0o755).unwrap();
        }
        fs::write(root.path().join("metadata.json"),br#"{"version":"0.2.0","phoenixdVersion":"0.9.3","stateFormat":"phoenixd-0.9.3","upgradeFrom":[]}"#).unwrap();
        let key = PKey::generate_ed25519().unwrap();
        let private = key.private_key_to_pem_pkcs8().unwrap();
        let public = key.public_key_to_pem().unwrap();
        let sha = pack(
            &root.path().join("payload"),
            &root.path().join("metadata.json"),
            &private,
            &public,
            &root.path().join("release"),
        )
        .unwrap();
        (root, private, public, sha)
    }
    fn resign(root: &Path, private: &[u8], mut manifest: Value, bundle: &[u8]) -> String {
        use openssl::sign::Signer;
        manifest["bundleSha256"] = hash(bundle).into();
        manifest["bundleSize"] = (bundle.len() as u64).into();
        let encoded = canonical(&manifest);
        let sha = hash(&encoded);
        let mut message = DOMAIN.to_vec();
        message.extend(encoded);
        let key = PKey::private_key_from_pem(private).unwrap();
        fs::write(root.join("release.json"),canonical(&json!({"manifest":manifest,"keyId":hash(&key.raw_public_key().unwrap()),"signature":hex::encode(Signer::new_without_digest(&key).unwrap().sign_oneshot_to_vec(&message).unwrap())}))).unwrap();
        fs::write(root.join("bundle.tar.gz"), bundle).unwrap();
        sha
    }
    #[test]
    fn signed_metadata_and_explicit_update_edges_are_checked() {
        let (root, private, public, _) = fixture();
        let release = root.path().join("release");
        let original =
            parse_json(&fs::read(release.join("release.json")).unwrap()).unwrap()["manifest"]
                .clone();
        let bundle = fs::read(release.join("bundle.tar.gz")).unwrap();
        for (key, value) in [
            ("platform", json!("linux-amd64")),
            ("unknown", json!(true)),
            (
                "upgradeFrom",
                json!([{"version":"0.3.0","releaseSha256":"a".repeat(64),"stateFormat":"phoenixd-0.9.3"}]),
            ),
        ] {
            let mut manifest = original.clone();
            manifest[key] = value;
            let sha = resign(&release, &private, manifest, &bundle);
            assert!(verify(&release, &public, &sha).is_err());
        }
        assert!(parse_json(br#"{"version":"0.1.0","version":"0.2.0"}"#).is_err());
        let mut e = parse_json(&fs::read(release.join("release.json")).unwrap()).unwrap();
        e["manifest"]["version"] = "0.4.0".into();
        let sha = hash(&canonical(&e["manifest"]));
        fs::write(release.join("release.json"), canonical(&e)).unwrap();
        assert!(verify(&release, &public, &sha).is_err());
    }
    #[test]
    fn signer_cannot_smuggle_links_duplicates_or_trailing_members() {
        let (root, private, public, _) = fixture();
        let release = root.path().join("release");
        let manifest =
            parse_json(&fs::read(release.join("release.json")).unwrap()).unwrap()["manifest"]
                .clone();
        let original = fs::read(release.join("bundle.tar.gz")).unwrap();
        let expanded = expand(&original, MAX_EXPANDED).unwrap();
        for kind in ["link", "duplicate", "trailing"] {
            let mut changed = expanded.clone();
            match kind {
                "link" => {
                    let header = tar::Header::from_byte_slice(&expanded[..512]);
                    let mut h = header.clone();
                    h.set_entry_type(tar::EntryType::Symlink);
                    h.set_link_name("/outside").unwrap();
                    h.set_cksum();
                    changed[..512].copy_from_slice(h.as_bytes());
                }
                "duplicate" => {
                    let mut bytes = expanded[..1024].to_vec();
                    bytes.extend(&expanded);
                    changed = bytes;
                }
                _ => changed.extend(b"unexpected"),
            }
            let mut encoder = flate2::GzBuilder::new()
                .mtime(0)
                .write(Vec::new(), flate2::Compression::default());
            use std::io::Write;
            encoder.write_all(&changed).unwrap();
            let archive = encoder.finish().unwrap();
            let sha = resign(&release, &private, manifest.clone(), &archive);
            assert!(verify(&release, &public, &sha).is_err(), "{kind}");
        }
    }
    #[test]
    fn pack_rejects_symlinks_and_wrong_architecture_without_output() {
        let (root, private, public, _) = fixture();
        let file = root.path().join("payload/lib/ld.so");
        fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink("../libexec/phoenixd", &file).unwrap();
        let pack_again = || {
            pack(
                &root.path().join("payload"),
                &root.path().join("metadata.json"),
                &private,
                &public,
                &root.path().join("new"),
            )
        };
        assert!(pack_again().is_err());
        assert!(!root.path().join("new").exists());
        fs::remove_file(&file).unwrap();
        let mut bytes = fs::read(root.path().join("payload/libexec/phoenixd")).unwrap();
        bytes[18] = 62;
        fs::write(&file, bytes).unwrap();
        assert!(pack_again().is_err());
    }
    #[test]
    fn decompression_stops_at_limit() {
        use std::io::Write;
        let mut e = flate2::GzBuilder::new().write(Vec::new(), flate2::Compression::default());
        e.write_all(&vec![0; 10000]).unwrap();
        let bytes = e.finish().unwrap();
        assert!(expand(&bytes, 100).is_err());
    }
    #[test]
    fn signed_output_is_deterministic_and_tampering_or_wrong_approval_fails() {
        let root = tempfile::tempdir().unwrap();
        let payload = root.path().join("payload");
        let mut elf = vec![0; 64];
        elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
        elf[18] = 183;
        for name in ["bin/nodana-phoenixd", "libexec/phoenixd", "lib/ld.so"] {
            let path = payload.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &elf).unwrap();
            mode(&path, 0o755).unwrap();
        }
        let metadata = root.path().join("metadata.json");
        fs::write(&metadata,br#"{"version":"0.1.0","phoenixdVersion":"0.9.3","stateFormat":"phoenixd-0.9.3","upgradeFrom":[]}"#).unwrap();
        let key = PKey::generate_ed25519().unwrap();
        let private = key.private_key_to_pem_pkcs8().unwrap();
        let public = key.public_key_to_pem().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        let sha = pack(&payload, &metadata, &private, &public, &first).unwrap();
        assert_eq!(
            sha,
            pack(&payload, &metadata, &private, &public, &second).unwrap()
        );
        for name in ["release.json", "bundle.tar.gz"] {
            assert_eq!(
                fs::read(first.join(name)).unwrap(),
                fs::read(second.join(name)).unwrap()
            );
        }
        assert!(verify(&first, &public, &"0".repeat(64)).is_err());
        assert!(
            verify(
                &first,
                &PKey::generate_ed25519()
                    .unwrap()
                    .public_key_to_pem()
                    .unwrap(),
                &sha
            )
            .is_err()
        );
        fs::write(first.join("bundle.tar.gz"), b"corrupt").unwrap();
        assert!(verify(&first, &public, &sha).is_err());
    }
}
