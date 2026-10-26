//! kadr.exe's icon, shown in Explorer, the Start menu and the taskbar.

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=data/windows/kadr.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("data/windows/kadr.ico");
        res.compile().expect("kadr.exe's resources");
    }
}
