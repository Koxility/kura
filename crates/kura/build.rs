fn main() {
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
