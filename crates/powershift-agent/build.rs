include!("../../build/windows_resources.rs");

fn main() {
    #[cfg(windows)]
    compile_windows_resources(
        "PowerShift",
        "PowerShift background agent",
        "powershift-agent.exe",
    );
}
