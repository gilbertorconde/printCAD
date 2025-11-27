use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=shaders/mesh.vert");
    println!("cargo:rerun-if-changed=shaders/mesh.frag");
    println!("cargo:rerun-if-changed=shaders/pick.vert");
    println!("cargo:rerun-if-changed=shaders/pick.frag");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    fs::create_dir_all(&out_dir).expect("failed to create OUT_DIR");

    compile_shader("mesh.vert", shaderc::ShaderKind::Vertex, &out_dir);
    compile_shader("mesh.frag", shaderc::ShaderKind::Fragment, &out_dir);
    compile_shader("pick.vert", shaderc::ShaderKind::Vertex, &out_dir);
    compile_shader("pick.frag", shaderc::ShaderKind::Fragment, &out_dir);
}

fn compile_shader(name: &str, kind: shaderc::ShaderKind, out_dir: &PathBuf) {
    let shaders_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest"));
    let source_path = shaders_dir.join("shaders").join(name);
    let source =
        fs::read_to_string(&source_path).unwrap_or_else(|e| panic!("read {} failed: {e}", name));

    let compiler = shaderc::Compiler::new().expect("failed to initialize shaderc compiler");
    let artifact = compiler
        .compile_into_spirv(&source, kind, name, "main", None)
        .unwrap_or_else(|e| panic!("shader compilation failed for {name}: {e}"));

    let output_path = out_dir.join(format!("{name}.spv"));
    fs::write(&output_path, artifact.as_binary_u8())
        .unwrap_or_else(|e| panic!("failed to write {:?}: {e}", output_path));
}
