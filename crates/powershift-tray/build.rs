fn main() {
    #[cfg(windows)]
    {
        embed_resource::compile("resources/powershift-tray.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("embed tray resources");
    }
}
