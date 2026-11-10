//! Desktop entry point; the native client is shared with Android.
fn main() {
    sta64::run(sta64::app::LaunchConfig::from_env());
}
