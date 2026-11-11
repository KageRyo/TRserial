//! Embeds the application icon and version information in Windows builds.
//! GPUI loads icon resource 1 for its windows and the taskbar.

#[path = "src/mark.rs"]
mod mark;

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/mark.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let icon = out.join("trserial.ico");
    fs::write(&icon, mark::ico(&[16, 20, 24, 32, 40, 48, 64, 256])).expect("write icon");
    let [major, minor, patch] = [
        "CARGO_PKG_VERSION_MAJOR",
        "CARGO_PKG_VERSION_MINOR",
        "CARGO_PKG_VERSION_PATCH",
    ]
    .map(|key| env::var(key).expect(key));
    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let icon = icon.display().to_string().replace('\\', "\\\\");
    // Numeric constants instead of winver.h keep the script free of SDK includes:
    // 0x40004 is VOS_NT_WINDOWS32, 0x1 is VFT_APP, and 0x0404/1200 is
    // Traditional Chinese (Taiwan) with Unicode strings.
    let script = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040404b0"
    BEGIN
      VALUE "FileDescription", "TRserial"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "trserial-desktop"
      VALUE "LegalCopyright", "MIT License"
      VALUE "OriginalFilename", "trserial-desktop.exe"
      VALUE "ProductName", "TRserial"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0404, 1200
  END
END
"#
    );
    let script_path = out.join("trserial.rc");
    fs::write(&script_path, script).expect("write resource script");
    embed_resource::compile(&script_path, embed_resource::NONE)
        .manifest_required()
        .expect("compile the Windows icon and version resources");
}
