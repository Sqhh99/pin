fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("resource/icon/pin.ico");
        if let Err(e) = res.compile() {
            eprintln!("winres failed: {e}");
        }
    }
}
