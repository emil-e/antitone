use antitone::{editor::Editor, platform::host};
use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "Antitone",
    bin_name = "antitone",
    version,
    about = "Edit camera RAW photographs and Antitone roll projects"
)]
struct Arguments {
    /// RAW photograph or saved roll to open
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Open an edit requested by the Lightroom plug-in
    #[arg(long, value_name = "REQUEST", conflicts_with = "file")]
    lightroom: Option<PathBuf>,

    /// Print the supported Lightroom transport version without opening a window
    #[arg(long, conflicts_with_all = ["file", "lightroom"])]
    lightroom_protocol_version: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    if arguments.lightroom_protocol_version {
        println!(
            "{}",
            antitone::editor::integrations::lightroom::PROTOCOL_VERSION
        );
        return Ok(());
    }

    let _session = antitone::logging::init();
    log::info!(libraw = antitone::raw::version().as_str(); "native_runtime");

    let initial = arguments.file;
    let external = arguments
        .lightroom
        .map(|request| antitone::editor::integrations::lightroom::read(&request))
        .transpose()?;
    let outcome = host::run("Antitone", move |device, queue, cx| {
        antitone::platform::install_icon()?;
        let mut editor = Editor::new(device, queue);
        if let Some(request) = external {
            editor.open_external(request, &cx);
        } else if let Some(path) = initial {
            editor.open_initial(path, &cx);
        }
        editor.restore_ui_preferences();
        Ok(editor)
    });

    match &outcome {
        Ok(frames) => log::info!(frames = *frames; "session_finished"),
        Err(error) => log::error!(error = error.to_string().as_str(); "session_failed"),
    }

    outcome.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, error::ErrorKind};

    #[test]
    fn argument_schema_is_consistent() {
        Arguments::command().debug_assert();
    }

    #[test]
    fn empty_launch_has_no_input() {
        let arguments = Arguments::try_parse_from(["antitone"]).unwrap();
        assert!(arguments.file.is_none());
        assert!(arguments.lightroom.is_none());
    }

    #[test]
    fn ordinary_and_lightroom_inputs_preserve_paths() {
        for path in ["scan.ARW", "röll 'one'/frame.dng", "saved roll.antitone"] {
            let arguments = Arguments::try_parse_from(["antitone", path]).unwrap();
            assert_eq!(arguments.file.as_deref(), Some(std::path::Path::new(path)));
            assert!(arguments.lightroom.is_none());

            let arguments = Arguments::try_parse_from(["antitone", "--lightroom", path]).unwrap();
            assert_eq!(
                arguments.lightroom.as_deref(),
                Some(std::path::Path::new(path))
            );
            assert!(arguments.file.is_none());
        }
    }

    #[test]
    fn rejects_missing_unknown_duplicate_or_conflicting_inputs() {
        for arguments in [
            &["antitone", "--lightroom"][..],
            &["antitone", "--unknown"][..],
            &["antitone", "one.ARW", "two.ARW"][..],
            &[
                "antitone",
                "--lightroom",
                "one.txt",
                "--lightroom",
                "two.txt",
            ][..],
            &["antitone", "one.ARW", "--lightroom", "request.txt"][..],
            &["antitone", "--lightroom", "request.txt", "one.ARW"][..],
        ] {
            assert!(
                Arguments::try_parse_from(arguments).is_err(),
                "accepted {arguments:?}"
            );
        }
    }

    #[test]
    fn help_and_version_exit_during_parsing() {
        for (flag, kind) in [
            ("--help", ErrorKind::DisplayHelp),
            ("-h", ErrorKind::DisplayHelp),
            ("--version", ErrorKind::DisplayVersion),
            ("-V", ErrorKind::DisplayVersion),
        ] {
            assert_eq!(
                Arguments::try_parse_from(["antitone", flag])
                    .unwrap_err()
                    .kind(),
                kind
            );
        }
    }

    #[test]
    fn protocol_discovery_is_exclusive_and_version_keeps_the_packaged_identity() {
        assert!(
            Arguments::try_parse_from(["antitone", "--lightroom-protocol-version"])
                .unwrap()
                .lightroom_protocol_version
        );
        for arguments in [
            &["antitone", "--lightroom-protocol-version", "roll.antitone"][..],
            &[
                "antitone",
                "--lightroom-protocol-version",
                "--lightroom",
                "request.json",
            ][..],
        ] {
            assert!(Arguments::try_parse_from(arguments).is_err());
        }
        let version = Arguments::try_parse_from(["antitone", "--version"])
            .unwrap_err()
            .to_string();
        assert!(version.starts_with("Antitone "));
    }

    #[test]
    fn end_of_options_allows_a_hyphenated_filename() {
        let arguments = Arguments::try_parse_from(["antitone", "--", "--lightroom"]).unwrap();
        assert_eq!(arguments.file, Some(PathBuf::from("--lightroom")));
        assert!(arguments.lightroom.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_are_preserved() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let path = OsString::from_vec(b"scan-\xff.ARW".to_vec());
        let arguments =
            Arguments::try_parse_from([OsString::from("antitone"), path.clone()]).unwrap();
        assert_eq!(arguments.file.unwrap().into_os_string(), path);

        let arguments = Arguments::try_parse_from([
            OsString::from("antitone"),
            OsString::from("--lightroom"),
            path.clone(),
        ])
        .unwrap();
        assert_eq!(arguments.lightroom.unwrap().into_os_string(), path);
    }
}
