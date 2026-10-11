use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/tessera.rc");
    println!("cargo:rerun-if-changed=assets/tessera.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var_os("CARGO_FEATURE_DESKTOP").is_none()
    {
        return;
    }

    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build output directory"));
    let target = env::var("TARGET").expect("Cargo target");
    let (compiler, resource, arguments) = if target.ends_with("-gnu") {
        let compiler = format!("{}-w64-mingw32-windres", target.split('-').next().unwrap());
        let resource = out.join("tessera.o");
        let arguments = vec![
            "-i".into(),
            "assets/tessera.rc".into(),
            "-O".into(),
            "coff".into(),
            "-o".into(),
            resource.as_os_str().to_owned(),
        ];
        (compiler, resource, arguments)
    } else if target.ends_with("-msvc") {
        let resource = out.join("tessera.res");
        let arguments = vec![
            "/nologo".into(),
            "/fo".into(),
            resource.as_os_str().to_owned(),
            "assets/tessera.rc".into(),
        ];
        ("rc.exe".into(), resource, arguments)
    } else {
        panic!("Unsupported Windows resource target: {target}");
    };

    // MSVC's SDK tools and include paths need discovery outside a developer prompt.
    let mut command = if target.ends_with("-msvc") {
        cc::windows_registry::find_tool(&target, &compiler)
            .map(|tool| tool.to_command())
            .unwrap_or_else(|| Command::new(&compiler))
    } else {
        Command::new(&compiler)
    };
    let output = command
        .current_dir(root)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| {
            panic!("Windows app icon requires {compiler} from the Windows SDK or PATH: {error}")
        });
    assert!(
        output.status.success(),
        "Windows resource compilation with {compiler} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("cargo:rustc-link-arg-bin=tessera={}", resource.display());
}
