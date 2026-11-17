//! Native libraries are always optimized: codec work must meet playback deadlines in debug too.
use std::{
    env, fs,
    hash::{Hash, Hasher},
    path::PathBuf,
};
fn main() {
    println!("cargo:rerun-if-changed=native");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    for name in ["aom", "avif"] {
        fs::read(format!("native/{name}.tar.gz"))
            .expect("vendored sources")
            .hash(&mut digest);
    }
    // A content-keyed directory prevents stale native sources when an archive is updated.
    let sources = out.join(format!("sources-{:016x}", digest.finish()));
    for name in ["aom", "avif"] {
        if !sources.join(name).join(".unpacked").exists() {
            fs::create_dir_all(&sources).expect("native source directory");
            let file =
                fs::File::open(format!("native/{name}.tar.gz")).expect("vendored codec sources");
            tar::Archive::new(flate2::read::GzDecoder::new(file))
                .unpack(&sources)
                .expect("unpack codec sources");
            fs::write(sources.join(name).join(".unpacked"), b"").expect("source marker");
        }
    }
    let mut cfg = cmake::Config::new("native");
    cfg.profile("Release")
        .define("SOURCES", &sources)
        .define("CMAKE_INSTALL_LIBDIR", "lib");
    if cfg!(windows) {
        cfg.define("CMAKE_C_FLAGS_RELEASE", "/O2 /Ob2 /DNDEBUG")
            .define("CMAKE_CXX_FLAGS_RELEASE", "/O2 /Ob2 /DNDEBUG");
        let perl = PathBuf::from("C:/Program Files/Git/usr/bin/perl.exe");
        if perl.exists() {
            cfg.define("PERL_EXECUTABLE", perl);
        }
    }
    if !cfg!(windows) {
        cfg.define("CMAKE_C_FLAGS_RELEASE", "-O3 -DNDEBUG")
            .define("CMAKE_CXX_FLAGS_RELEASE", "-O3 -DNDEBUG");
    }
    let built = cfg.build();
    println!(
        "cargo:rustc-link-search=native={}",
        built.join("lib").display()
    );
    for library in ["dt_render_codec", "avif_internal", "aom"] {
        println!("cargo:rustc-link-lib=static={library}");
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        println!("cargo:rustc-link-lib=stdc++");
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=pthread");
    }
}
