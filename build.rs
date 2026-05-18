fn main() {
    let lib = pkg_config::probe_library("rubberband").unwrap();
    for path in &lib.link_paths {
        println!("cargo:rustc-link-search={}", path.display());
    }
}
