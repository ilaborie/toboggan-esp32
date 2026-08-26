fn main() {
    embuild::espidf::sysenv::output();

    // `config::env` bakes these in with `env!`, so they are build inputs: without
    // this, switching between the device and simulator values leaves the source
    // unchanged and cargo reuses the previous binary.
    for var in [
        "WIFI_SSID",
        "WIFI_PASSWORD",
        "TOBOGGAN_HOST",
        "TOBOGGAN_HOST_FALLBACK",
        "TOBOGGAN_PORT",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
}
