#[cfg(windows)]
pub fn compile_windows_resources(product_name: &str, description: &str, file_name: &str) {
    use std::path::PathBuf;

    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let numeric_version = numeric_version(&version);
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let icon = manifest_dir
        .join("../../src-tauri/icons/icon.ico")
        .canonicalize()
        .expect("PowerShift icon");
    let icon = icon.display().to_string().replace('\\', "\\\\");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"))
        .join(format!("{file_name}.rc"));

    let resource = format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {numeric_version}
PRODUCTVERSION {numeric_version}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "4ismael1"
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "{file_name}"
      VALUE "LegalCopyright", "Copyright (c) 2026 4ismael1"
      VALUE "OriginalFilename", "{file_name}"
      VALUE "ProductName", "{product_name}"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );

    std::fs::write(&output, resource).expect("write Windows resources");
    println!("cargo:rerun-if-changed={}", icon.replace("\\\\", "\\"));
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
    embed_resource::compile(&output, embed_resource::NONE)
        .manifest_optional()
        .expect("embed Windows resources");
}

#[cfg(windows)]
fn numeric_version(version: &str) -> String {
    let mut parts = version
        .split('-')
        .next()
        .unwrap_or(version)
        .split('.')
        .map(|part| part.parse::<u16>().expect("numeric package version"))
        .collect::<Vec<_>>();
    parts.resize(4, 0);
    parts.truncate(4);
    parts
        .into_iter()
        .map(|part| part.to_string())
        .collect::<Vec<_>>()
        .join(",")
}
