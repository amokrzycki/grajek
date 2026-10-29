fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=assets/windows.rc");
    println!("cargo:rerun-if-changed=assets/brand/grajek-windows.ico");
    embed_resource::compile_for(
        "assets/windows.rc",
        ["grajek"],
        embed_resource::ParamsIncludeDirs(["assets"]),
    )
    .manifest_required()?;
    Ok(())
}
