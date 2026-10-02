//! Embed the icon, application manifest, and version info into `pin.exe`.

/// Per-monitor-v2 DPI awareness (window rects and our windows in physical
/// pixels), Windows 10/11 compatibility, and Common Controls v6.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2, PerMonitor</dpiAwareness>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=resource/icon/pin.ico");

    // `cfg(windows)` here would describe the build *host*; check the target.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon("resource/icon/pin.ico")
        .set_manifest(MANIFEST)
        .set("FileDescription", "Pin - keep any window on top")
        .set("ProductName", "Pin");
    if let Err(e) = res.compile() {
        // Typically a missing resource compiler when cross-compiling. The
        // app still works (it opts into DPI awareness at runtime), but has
        // no icon or version info; the release workflow checks for both.
        println!("cargo:warning=failed to embed Windows resources: {e}");
    }
}
