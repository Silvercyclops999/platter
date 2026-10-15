fn main() {
    // The Discord application id is read at compile time.
    println!("cargo:rerun-if-env-changed=PLATTER_DISCORD_APP_ID");
    // The macOS keychain owner is the team the release is signed with.
    println!("cargo:rerun-if-env-changed=APPLE_TEAM_ID");
    tauri_build::build()
}
