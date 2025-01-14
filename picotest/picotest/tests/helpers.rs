use std::process::{Child, Command, Output};
use std::time::{Duration, Instant};

pub fn wait_for_proc(proc: &mut Child, timeout: Duration) {
    let start_time = Instant::now();

    loop {
        assert!(
            start_time.elapsed() < timeout,
            "Process hanging for too long"
        );

        match proc.try_wait().unwrap() {
            Some(_) => {
                break;
            }
            None => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

pub fn build_plugin(path: &str) -> std::io::Result<Output> {
    Command::new("cargo")
        .args(vec!["build"])
        .current_dir(path)
        .output()
}
