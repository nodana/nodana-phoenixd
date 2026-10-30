use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=NODANA_CA_BUNDLE");
    println!("cargo:rerun-if-env-changed=NODANA_RELEASE");
    let bytes = match env::var_os("NODANA_CA_BUNDLE") {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", PathBuf::from(&path).display());
            let bytes = fs::read(path).expect("read bundled CA certificates");
            assert!(bytes.len() <= 2 * 1024 * 1024);
            let text = std::str::from_utf8(&bytes).expect("PEM certificate bundle");
            assert!(text.contains("-----BEGIN CERTIFICATE-----"));
            assert!(!text.contains("-----BEGIN TRUSTED CERTIFICATE-----"));
            bytes
        }
        None => {
            assert!(
                env::var_os("NODANA_RELEASE").is_none(),
                "release requires NODANA_CA_BUNDLE"
            );
            Vec::new()
        }
    };
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("ca-bundle.pem"),
        bytes,
    )
    .expect("embed CA certificates");
}
