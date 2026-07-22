fn main() {
    #[cfg(target_os = "windows")]
    let _ = embed_resource::compile("packaging/windows/manifest.rc", embed_resource::NONE);
}
