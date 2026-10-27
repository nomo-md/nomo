fn main() {
    #[cfg(windows)]
    {
        use std::{env, fs, path::PathBuf};
        println!("cargo:rerun-if-changed=assets/app/icon.ico");
        println!("cargo:rerun-if-changed=assets/app/windows.manifest");
        println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
        let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let version = env::var("CARGO_PKG_VERSION").unwrap();
        let numeric = format!("{},{},{},0", env::var("CARGO_PKG_VERSION_MAJOR").unwrap(),
            env::var("CARGO_PKG_VERSION_MINOR").unwrap(), env::var("CARGO_PKG_VERSION_PATCH").unwrap());
        let icon = root.join("assets/app/icon.ico").to_string_lossy().replace('\\', "/");
        let manifest = root.join("assets/app/windows.manifest").to_string_lossy().replace('\\', "/");
        let resource = out.join("nomo.rc");
        fs::write(&resource, format!(r#"
#pragma code_page(65001)
1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "Nomo"
      VALUE "ProductName", "Nomo"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "nomo.exe"
      VALUE "LegalCopyright", "AGPL-3.0-or-later"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#)).expect("write Windows resource");
        embed_resource::compile(resource, embed_resource::NONE)
            .manifest_required().expect("compile Nomo Windows resources");
    }
}
