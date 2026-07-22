fn main() {
    let libraw = pkg_config::Config::new()
        .atleast_version("0.21")
        .probe("libraw")
        .expect("libraw not found via pkg-config; enter the nix dev shell");

    let include_args: Vec<String> = libraw
        .include_paths
        .iter()
        .map(|p| format!("-I{}", p.display()))
        .collect();

    let bindings = bindgen::Builder::default()
        .header_contents("wrapper.h", "#include <libraw/libraw.h>")
        .clang_args(&include_args)
        .allowlist_function("libraw_.*")
        .allowlist_type("libraw_.*")
        .allowlist_var("LIBRAW_.*")
        .derive_default(false)
        .derive_debug(false)
        .layout_tests(false)
        .generate()
        .expect("failed to generate libraw bindings");

    let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("failed to write bindings");

    println!("cargo:rerun-if-changed=build.rs");
}
