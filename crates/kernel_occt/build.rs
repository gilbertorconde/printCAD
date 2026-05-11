use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=cpp/step_loader.cpp");
    println!("cargo:rerun-if-changed=cpp/step_loader.h");
    println!("cargo:rerun-if-env-changed=OCCT_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=OCCT_LIB_DIR");

    // Find OCCT headers. Allow override via env var, otherwise probe a few
    // common system locations.
    let include_candidates: Vec<PathBuf> = if let Ok(env_dir) = std::env::var("OCCT_INCLUDE_DIR") {
        vec![PathBuf::from(env_dir)]
    } else {
        vec![
            PathBuf::from("/usr/include/opencascade"),
            PathBuf::from("/usr/local/include/opencascade"),
            PathBuf::from("/opt/opencascade/include"),
        ]
    };

    let occt_include = include_candidates
        .into_iter()
        .find(|p| p.join("STEPControl_Reader.hxx").exists())
        .unwrap_or_else(|| {
            panic!(
                "Unable to locate OpenCASCADE headers. Set OCCT_INCLUDE_DIR \
                 to the directory containing STEPControl_Reader.hxx."
            );
        });

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("cpp/step_loader.cpp")
        .include("cpp")
        .include(&occt_include)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-Wno-deprecated-declarations")
        // OCCT 7.6+ requires this define for proper Handle() expansion.
        .define("HAVE_NO_DLL", None);
    build.compile("printcad_occt_shim");

    if let Ok(lib_dir) = std::env::var("OCCT_LIB_DIR") {
        println!("cargo:rustc-link-search=native={}", lib_dir);
    }

    // OpenCASCADE 7.6+ split STEP support out of the legacy `TKSTEP` library
    // into a unified `TKDESTEP` data-exchange module. Both names are listed so
    // the build works against either generation when available.
    let occt_libs = [
        "TKDESTEP", // STEP + STEPCAFControl (geometry + colours in DECAF doc).
        "TKXSBase",
        "TKXCAF", // XCAF document / colour attributes on shapes.
        "TKVCAF",
        "TKLCAF",
        "TKCAF",
        "TKMesh",
        "TKBRep",
        "TKTopAlgo",
        "TKGeomAlgo",
        "TKGeomBase",
        "TKG3d",
        "TKG2d",
        "TKMath",
        "TKernel",
    ];
    for name in occt_libs {
        println!("cargo:rustc-link-lib=dylib={name}");
    }
}
