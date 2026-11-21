fn main() {
    // The commit in the footer: given by the image build, else asked from git.
    println!("cargo::rerun-if-env-changed=KURA_COMMIT");
    println!("cargo::rerun-if-changed=../../.git/HEAD");
    let commit = std::env::var("KURA_COMMIT")
        .ok()
        .filter(|commit| !commit.trim().is_empty())
        .or_else(|| {
            let out = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        })
        .map(|commit| commit.trim().chars().take(7).collect::<String>())
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo::rustc-env=KURA_COMMIT={commit}");

    // The web app is embedded from this folder, which is absent until `web/` was built once.
    std::fs::create_dir_all("../../web/dist").expect("web/dist can be created");

    // The version in the top bar is this crate's. Refusing to build when web/package.json
    // says something else keeps a release from shipping two different numbers.
    println!("cargo::rerun-if-changed=../../web/package.json");
    let package = std::fs::read_to_string("../../web/package.json")
        .expect("web/package.json is part of the repository");
    let web = package
        .lines()
        .find_map(|line| line.trim().strip_prefix("\"version\":"))
        .map(|value| value.trim().trim_end_matches(',').trim_matches('"'))
        .expect("web/package.json has a version");
    let server = env!("CARGO_PKG_VERSION");
    assert!(
        web == server,
        "web/package.json has version {web}, Cargo.toml has {server}; set both to the same"
    );
}
