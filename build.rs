fn main() {
    match pkg_config::probe_library("rubberband") {
        Ok(lib) => {
            for path in &lib.link_paths {
                println!("cargo:rustc-link-search={}", path.display());
            }
        }
        Err(_) => {
            // pkg-config not available — link to the versioned .so directly.
            // This handles the case where the runtime lib is installed but not the -dev package.
            let search_dirs = [
                "/usr/lib/x86_64-linux-gnu",
                "/usr/local/lib",
                "/usr/lib",
            ];
            let versioned = "librubberband.so.2";
            for dir in &search_dirs {
                let path = std::path::Path::new(dir).join(versioned);
                if path.exists() {
                    println!("cargo:rustc-link-search={}", dir);
                    // Pass the versioned filename directly to the linker.
                    println!("cargo:rustc-link-arg=-Wl,-l:librubberband.so.2");
                    return;
                }
            }
            // Last resort: hope the linker can find it on its own.
            println!("cargo:rustc-link-lib=rubberband");
        }
    }
}
