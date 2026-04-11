fn main() {
    let host = match std::env::var("BUILD_ENV").as_deref() {
        Ok("dev") => "dev.genai.stargate.toyota",
        Ok("stg") => "stg.genai.stargate.toyota",
        _         => "genai.stargate.toyota",
    };
    println!("cargo:rustc-env=AGC_DEFAULT_HOST={host}");
    // Re-run if BUILD_ENV changes.
    println!("cargo:rerun-if-env-changed=BUILD_ENV");
}
