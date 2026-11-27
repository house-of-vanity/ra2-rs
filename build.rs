fn main() {
    println!("cargo::rerun-if-changed=assets/icons/windows.rc");
    println!("cargo::rerun-if-changed=assets/icons/ra2-rs.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    embed_resource::compile_for(
        "assets/icons/windows.rc",
        [env!("CARGO_PKG_NAME")],
        embed_resource::ParamsIncludeDirs(["assets/icons"]),
    )
    .manifest_required()
    .expect("failed to embed the Windows application icon");
}
