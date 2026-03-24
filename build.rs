fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=vigem_src/src/ViGEmClient.cpp");

    cc::Build::new()
        .cpp(true)
        .file("vigem_src/src/ViGEmClient.cpp")
        .include("vigem_src/include")
        .include("vigem_src/include/km")
        .compile("vigemclient");

    println!("cargo:rustc-link-lib=setupapi");

    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_manifest_file("app.manifest");
        res.compile().unwrap();
    }
}
