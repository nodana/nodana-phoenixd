use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
extern "C" fn interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}
const NIX: &str = "nixos/nix:2.28.5";
const ALPINE: &str = "alpine:3.22";
struct Docker {
    context: String,
    owner: String,
    names: Vec<String>,
    images: Vec<(String, bool)>,
    cleaned: bool,
    payload: Option<PathBuf>,
}
impl Docker {
    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new("docker");
        c.args(["--context", &self.context]).args(args);
        c
    }
    fn text(&self, args: &[&str]) -> Result<String> {
        ensure!(!INTERRUPTED.load(Ordering::Relaxed), "build interrupted");
        let o = self.command(args).output()?;
        ensure!(
            o.status.success(),
            "Docker {} failed (exit {:?})",
            args.first().unwrap_or(&"operation"),
            o.status.code()
        );
        Ok(String::from_utf8(o.stdout)?.trim().into())
    }
    fn run(&self, args: &[&str]) -> Result<()> {
        self.text(args)?;
        Ok(())
    }
    fn usage(&self, builder: &str) -> Result<u64> {
        let image: serde_json::Value =
            serde_json::from_str(&self.text(&["image", "inspect", NIX])?)?;
        let container: serde_json::Value =
            serde_json::from_str(&self.text(&["inspect", "--size", builder])?)?;
        image[0]["Size"]
            .as_u64()
            .and_then(|image| {
                container[0]["SizeRw"]
                    .as_u64()
                    .and_then(|w| image.checked_add(w))
            })
            .context("missing Docker build size")
    }
    fn budget(&self, builder: &str) -> Result<()> {
        for _ in 0..5 {
            if let Ok(used) = self.usage(builder) {
                ensure!(used <= 15 * 1024 * 1024 * 1024, "build exceeded 15 GiB");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        self.run(&["pause", builder])?;
        let result = self.usage(builder);
        let resumed = self.run(&["unpause", builder]);
        resumed?;
        ensure!(
            result.context("unable to measure build storage")? <= 15 * 1024 * 1024 * 1024,
            "build exceeded 15 GiB"
        );
        Ok(())
    }
    fn cleanup(&mut self) -> Result<()> {
        let mut failed = false;
        for name in &self.names {
            let o = self
                .command(&[
                    "inspect",
                    name,
                    "--format",
                    "{{ index .Config.Labels \"nodana.build.owner\" }}",
                ])
                .output()?;
            if o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == self.owner {
                failed |= !self
                    .command(&["rm", "--force", "--volumes", name])
                    .output()?
                    .status
                    .success();
            }
        }
        for (image, existed) in &self.images {
            if !existed
                && self
                    .command(&["image", "inspect", image])
                    .output()?
                    .status
                    .success()
            {
                failed |= !self
                    .command(&["image", "rm", image])
                    .output()?
                    .status
                    .success();
            }
        }
        self.cleaned = true;
        ensure!(!failed, "task-owned Docker cleanup needs attention");
        Ok(())
    }
}
impl Drop for Docker {
    fn drop(&mut self) {
        if let Some(payload) = &self.payload {
            if let Err(error) = fs::remove_dir_all(payload) {
                eprintln!("Partial payload cleanup failed: {error}");
            }
        }
        if !self.cleaned && self.cleanup().is_err() {
            eprintln!("Task-owned Docker cleanup needs attention");
        }
    }
}
fn p(path: &Path) -> Result<&str> {
    path.to_str().context("non-UTF8 build path")
}
pub fn build(source: &Path, output: &Path, context: &str) -> Result<()> {
    ensure!(
        ["desktop-linux", "default"].contains(&context),
        "select a local native Docker context"
    );
    ensure!(
        std::env::var_os("DOCKER_HOST").is_none() && std::env::var_os("DOCKER_CONTEXT").is_none(),
        "unset Docker overrides"
    );
    unsafe {
        libc::signal(libc::SIGTERM, interrupt as libc::sighandler_t);
        libc::signal(libc::SIGINT, interrupt as libc::sighandler_t);
    }
    let source = source.canonicalize()?;
    ensure!(
        output.symlink_metadata().is_err(),
        "choose a new output directory"
    );
    fs::create_dir_all(output)?;
    let output = output.canonicalize()?;
    let mut random = [0; 12];
    openssl::rand::rand_bytes(&mut random)?;
    let owner = format!("nodana-bundle-{}", hex::encode(random));
    let builder = format!("{owner}-build");
    let smoke = format!("{owner}-smoke");
    let mut docker = Docker {
        context: context.into(),
        owner: owner.clone(),
        names: vec![builder.clone(), smoke.clone()],
        images: vec![],
        cleaned: false,
        payload: None,
    };
    ensure!(
        ["aarch64", "arm64"].contains(
            &docker
                .text(&["info", "--format", "{{.Architecture}}"])?
                .as_str()
        ),
        "native ARM Docker is required"
    );
    for image in [NIX, ALPINE] {
        let existed = docker
            .command(&["image", "inspect", image])
            .output()?
            .status
            .success();
        docker.images.push((image.into(), existed));
        if !existed {
            docker.run(&["pull", "--quiet", "--platform", "linux/arm64", image])?;
        }
    }
    let label = format!("nodana.build.owner={owner}");
    docker.run(&[
        "run",
        "-d",
        "--platform",
        "linux/arm64",
        "--name",
        &builder,
        "--label",
        &label,
        "--memory",
        "6g",
        "--cpus",
        "4",
        NIX,
        "sh",
        "-c",
        "sleep infinity",
    ])?;
    ensure!(
        docker.text(&["exec", &builder, "uname", "-m"])? == "aarch64",
        "builder must run native ARM64"
    );
    let staging = tempfile::tempdir()?;
    for name in ["build.nix", "bundle.nix", "phoenixd.nix", "release.json"] {
        fs::copy(source.join(name), staging.path().join(name))?;
    }
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "src/main.rs",
        "src/trust.rs",
    ] {
        let target = staging.path().join("launcher").join(name);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(source.join("launcher").join(name), target)?;
    }
    docker.run(&[
        "cp",
        &format!("{}/.", p(staging.path())?),
        &format!("{builder}:/source"),
    ])?;
    let log = fs::File::create(output.join("build.log"))?;
    let mut child = docker
        .command(&[
            "exec",
            &builder,
            "nix-build",
            "/source/build.nix",
            "--option",
            "sandbox",
            "false",
            "--option",
            "max-jobs",
            "2",
            "--out-link",
            "/result",
        ])
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(1800);
    let result = (|| -> Result<()> {
        loop {
            if let Some(status) = child.try_wait()? {
                ensure!(status.success(), "Nix build failed; inspect build.log");
                break;
            }
            ensure!(
                Instant::now() < deadline && !INTERRUPTED.load(Ordering::Relaxed),
                "build timed out or interrupted"
            );
            docker.budget(&builder)?;
            std::thread::sleep(Duration::from_secs(3));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result?;
    docker.run(&[
        "exec",
        &builder,
        "sh",
        "-c",
        "mkdir /export && cp -rL /result/. /export/ && chmod -R u+w /export",
    ])?;
    let payload = output.join("payload");
    fs::create_dir(&payload)?;
    docker.payload = Some(payload.clone());
    docker.run(&["cp", &format!("{builder}:/export/."), p(&payload)?])?;
    docker.run(&[
        "run",
        "-d",
        "--platform",
        "linux/arm64",
        "--network",
        "none",
        "--name",
        &smoke,
        "--label",
        &label,
        "--memory",
        "512m",
        ALPINE,
        "sh",
        "-c",
        "sleep infinity",
    ])?;
    docker.run(&["cp", p(&payload)?, &format!("{smoke}:/relocated")])?;
    let version = docker.text(&[
        "exec",
        &smoke,
        "/relocated/bin/nodana-phoenixd",
        "--nodana-version",
    ])?;
    let phoenix = docker.text(&[
        "exec",
        &smoke,
        "env",
        "HOME=/tmp",
        "PHOENIX_DATADIR=/tmp/version",
        "TZ=UTC",
        "/relocated/bin/nodana-phoenixd",
        "--version",
    ])?;
    let metadata = crate::wire::parse_json(&fs::read(source.join("release.json"))?)?;
    ensure!(
        metadata["version"] == version
            && phoenix.contains(
                metadata["phoenixdVersion"]
                    .as_str()
                    .context("missing Phoenixd version")?
            ),
        "unexpected runtime versions"
    );
    fs::write(
        output.join("build-result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"platform":"linux-arm64","nodanaVersion":version,"phoenixdVersionOutput":phoenix,"relocationSmokeTest":"passed","networkDuringSmokeTest":"none","walletStarted":false}),
        )?,
    )?;
    docker.payload = None;
    docker.cleanup()?;
    println!("ARM bundle build and relocation checks passed");
    Ok(())
}
