fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../resources/windows/myterm.ico");
        res.set("ProductName", "MyTerm");
        res.set("FileDescription", "MyTerm - My Terminal");
        res.set("LegalCopyright", "Copyright (c) 2025 MyTerm");
        res.compile().expect("Failed to compile Windows resources");
    }
}
