fn main() {
    #[cfg(target_os = "windows")]
    {
        // Embed Windows icon
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/logo.ico");
        res.compile().unwrap();
    }
}
