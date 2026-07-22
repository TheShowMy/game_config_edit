use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, Eq, PartialEq)]
struct RevealCommand {
    program: &'static str,
    arguments: Vec<PathBuf>,
}

pub fn reveal_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "Show in Explorer"
    } else if cfg!(target_os = "macos") {
        "Show in Finder"
    } else {
        "Show in file manager"
    }
}

pub fn reveal_in_file_manager(path: &Path) -> io::Result<()> {
    let command = reveal_command(path)?;
    Command::new(command.program)
        .args(command.arguments)
        .spawn()
        .map(|_| ())
}

fn reveal_command(path: &Path) -> io::Result<RevealCommand> {
    #[cfg(target_os = "windows")]
    {
        return Ok(RevealCommand {
            program: "explorer.exe",
            arguments: vec![PathBuf::from(format!("/select,{}", path.display()))],
        });
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(RevealCommand {
            program: "open",
            arguments: vec![PathBuf::from("-R"), path.to_path_buf()],
        });
    }
    #[allow(unreachable_code)]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the current platform has no supported file manager integration",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn explorer_receives_the_selection_as_one_argument() {
        let command = reveal_command(Path::new(r"C:\configs with spaces\heroes.csv")).unwrap();

        assert_eq!(command.program, "explorer.exe");
        assert_eq!(
            command.arguments,
            vec![PathBuf::from(r"/select,C:\configs with spaces\heroes.csv")]
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn finder_receives_a_separate_reveal_argument() {
        let command = reveal_command(Path::new("/tmp/configs with spaces/heroes.csv")).unwrap();

        assert_eq!(command.program, "open");
        assert_eq!(
            command.arguments,
            vec![
                PathBuf::from("-R"),
                PathBuf::from("/tmp/configs with spaces/heroes.csv")
            ]
        );
    }
}
