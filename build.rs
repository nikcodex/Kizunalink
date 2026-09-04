fn main() {
    // 1. Build timestamp in epoch milliseconds
    let build_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string());
    println!("cargo:rustc-env=BUILD_TIME={}", build_time);

    // 2. Git commit hash
    let git_commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                String::from_utf8(out.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_COMMIT={}", git_commit);

    // 3. Git branch name
    let git_branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                String::from_utf8(out.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .or_else(|| std::env::var("GITHUB_REF_NAME").ok())
        .unwrap_or_else(|| "main".to_string());
    println!("cargo:rustc-env=GIT_BRANCH={}", git_branch);

    // 4. Git commit timestamp in epoch milliseconds
    let git_commit_time = std::process::Command::new("git")
        .args(["log", "-1", "--format=%ct"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                String::from_utf8(out.stdout)
                    .ok()
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .map(|secs| (secs * 1000).to_string())
            } else {
                None
            }
        })
        .unwrap_or(build_time);
    println!("cargo:rustc-env=GIT_COMMIT_TIME={}", git_commit_time);
}
