fn main() {
    // Dynamically link to the system libopus
    pkg_config::Config::new().probe("opus").expect("libopus must be installed");
}
