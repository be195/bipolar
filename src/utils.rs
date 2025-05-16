use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;
use std::process::ExitStatus;

pub fn copy_dir_recursive(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();

    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;

        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(src_path, dst_path)?;
        } else {
            fs::copy(src_path, dst_path)?;
        }
    }

    Ok(())
}

pub fn run_command_string(cmd_str: &str, working_dir: &str) -> ExitStatus {
    #[cfg(unix)]
    let mut command = {
        let mut cmd = Command::new("sh");
        cmd.arg("-c");
        cmd
    };

    #[cfg(windows)]
    let mut command = {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C");
        cmd
    };

    command
        .arg(cmd_str)
        .current_dir(working_dir)
        .status()
        .expect("failed to execute command")
}
