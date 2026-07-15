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

    let occt_lib_dir = std::env::var("OCCT_LIB_DIR").ok();
    if let Some(lib_dir) = &occt_lib_dir {
        println!("cargo:rustc-link-search=native={}", lib_dir);
    }

    // OpenCASCADE 7.8 merged the legacy `TKSTEP*` libraries into a unified
    // `TKDESTEP` data-exchange module. Probe the library directories so the
    // build links whichever generation is installed.
    let mut lib_search_dirs: Vec<PathBuf> = occt_lib_dir.iter().map(PathBuf::from).collect();
    lib_search_dirs.extend(
        [
            "/usr/lib",
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib64",
            "/usr/local/lib",
            "/opt/opencascade/lib",
        ]
        .iter()
        .map(PathBuf::from),
    );
    let has_lib = |name: &str| {
        lib_search_dirs
            .iter()
            .any(|dir| dir.join(format!("lib{name}.so")).exists())
    };

    let step_libs: &[&str] = if has_lib("TKDESTEP") {
        // STEP + STEPCAFControl (geometry + colours in DECAF doc).
        &["TKDESTEP"]
    } else {
        // OCCT <= 7.7 legacy naming (TKXDESTEP carries STEPCAFControl).
        &[
            "TKXDESTEP",
            "TKSTEP",
            "TKSTEP209",
            "TKSTEPAttr",
            "TKSTEPBase",
        ]
    };

    let occt_libs = [
        "TKXSBase",
        "TKXCAF", // XCAF document / colour attributes on shapes.
        "TKVCAF",
        "TKLCAF",
        "TKCAF",
        "TKMesh",
        "TKBRep",
        "TKPrim",      // BRepPrimAPI_MakePrism (sketch extrusion).
        "TKBO",        // BRepAlgoAPI_Fuse / BRepAlgoAPI_Cut booleans.
        "TKBool",      // Boolean-op support toolkit used by TKBO.
        "TKShHealing", // ShapeFix_Face / ShapeUpgrade_UnifySameDomain.
        "TKTopAlgo",
        "TKGeomAlgo",
        "TKGeomBase",
        "TKG3d",
        "TKG2d",
        "TKMath",
        "TKernel",
    ];
    for name in step_libs.iter().copied().chain(occt_libs) {
        println!("cargo:rustc-link-lib=dylib={name}");
    }
}
