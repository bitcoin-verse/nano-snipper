fn main() {
    // Embed manifest and icon into the executable
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_manifest_file("resources/app.manifest");

        // Icon — shared from workspace root
        let icon_path = "../../resources/app.ico";
        if std::path::Path::new(icon_path).exists() {
            res.set_icon(icon_path);
        }

        if let Err(e) = res.compile() {
            eprintln!("Warning: failed to embed resources: {e}");
        }
    }
}
