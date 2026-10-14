fn main() {
    // The Discord application id is read at compile time.
    println!("cargo:rerun-if-env-changed=PLATTER_DISCORD_APP_ID");
    tauri_build::build()
}
