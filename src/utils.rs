use std::fs;
use std::io;
use std::path::Path;
use std::process::Child;
use std::process::Command;

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

pub fn run_command_string(cmd_str: &str, working_dir: &str, asynch: bool) -> Result<Child, Box<dyn std::error::Error>> {
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
        .current_dir(working_dir);

    let mut child = command.spawn()?;
    if asynch {
        Ok(child)
    } else {
        child.wait()?;
        Ok(child)
    }
}

pub fn create_dir_symlink(original: &str, link: &str) -> io::Result<()> {
    let original_path = Path::new(original);
    let link_path = Path::new(link);

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(original_path, link_path)
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;
        symlink_dir(original_path, link_path)
    }
}

pub fn create_symlink_force(original: &str, link: &str) -> io::Result<()> {
    if Path::new(link).exists() {
        fs::remove_dir_all(link)?;
    }

    create_dir_symlink(original, link)
}
