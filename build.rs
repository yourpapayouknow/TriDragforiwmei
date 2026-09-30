fn main() {
    // Compiles app.rc, which embeds the application icon and the side-by-side
    // manifest. Rebuild whenever either input changes.
    let _ = embed_resource::compile("app.rc", embed_resource::NONE);
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=tridragforiwmei.exe.manifest");
}
