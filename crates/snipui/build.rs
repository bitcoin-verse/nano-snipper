fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winresource::WindowsResource::new();

        let manifest = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
</assembly>"#;

        res.set_manifest(manifest);

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
