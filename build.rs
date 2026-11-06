fn main() {
    println!("cargo:rerun-if-changed=assets/RSnap.ico");
    println!("cargo:rerun-if-changed=app.manifest");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/RSnap.ico");
    res.set_manifest_file("app.manifest");
    res.set("ProductName", "RSnap");
    res.set("FileDescription", "RSnap");
    res.compile().expect("failed to compile Windows resources");
}
