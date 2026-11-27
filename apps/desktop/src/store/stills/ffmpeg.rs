//! The still frames from the tools of the `FFmpeg` project, `ffprobe` for the length and
//! `ffmpeg` for each frame, where they are installed: in the `PATH`'s absolute folders, or where
//! Homebrew and `MacPorts` put them (an app opened from the Finder gets a short `PATH`). Send
//! waits for the frames, so each run has a deadline ([`PER_RUN`], [`IN_ALL`] for a video's runs
//! together): a run that hangs (a damaged file, a stalled network mount) is killed and the video
//! goes with the frames made so far.

use std::ffi::OsString;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::{LONGEST_SIDE, StopFlag, frame_times};

/// The folders `ffmpeg` is looked for in after the `PATH`.
const FOLDERS: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"];

/// How long one run of `ffprobe` or `ffmpeg` may take before it is killed.
const PER_RUN: Duration = Duration::from_secs(10);

/// How long a video's runs may take together.
const IN_ALL: Duration = Duration::from_secs(30);

/// How often a run is looked in on.
const POLL: Duration = Duration::from_millis(20);

/// The still frames of the video at `path` from the tools found on the `PATH` or in
/// [`FOLDERS`]; none without them. It blocks: call it off the UI thread.
pub(super) fn grab(path: &Path, stop: &StopFlag) -> Vec<Vec<u8>> {
    Tools::find(&folders(std::env::var_os("PATH"))).map_or_else(
        || {
            log::info!("no FFmpeg: {} goes without still frames", path.display());
            Vec::new()
        },
        |tools| tools.grab(path, stop, Limits::default()),
    )
}

/// A video's length as `ffprobe` prints it, in seconds.
#[must_use]
fn parse_duration(output: &str) -> Option<f64> {
    output
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
}

/// The folders to look for the tools in: the absolute ones of `path` (a `PATH` value; a
/// relative entry would run whatever the working folder holds), then [`FOLDERS`].
fn folders(path: Option<OsString>) -> Vec<PathBuf> {
    path.map_or_else(Vec::new, |path| {
        std::env::split_paths(&path).collect::<Vec<_>>()
    })
    .into_iter()
    .filter(|folder| folder.is_absolute())
    .chain(FOLDERS.iter().map(PathBuf::from))
    .collect()
}

/// The input argument for the file at `path`: through the `file:` protocol, so that neither
/// a `name:` prefix nor a leading `-` is read as anything but a file name.
fn input(path: &Path) -> OsString {
    let mut input = OsString::from("file:");
    input.push(path);
    input
}

/// The deadlines a video's runs have.
#[derive(Clone, Copy, Debug)]
struct Limits {
    /// For each run.
    per_run: Duration,
    /// For all of them.
    in_all: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_run: PER_RUN,
            in_all: IN_ALL,
        }
    }
}

/// The `FFmpeg` tools, where they are installed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tools {
    ffprobe: PathBuf,
    ffmpeg: PathBuf,
}

impl Tools {
    /// The tools in `folders`, the first of each found.
    fn find(folders: &[PathBuf]) -> Option<Self> {
        let tool = |name: &str| {
            let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
            folders
                .iter()
                .map(|folder| folder.join(&file))
                .find(|candidate| candidate.is_file())
        };
        Some(Self {
            ffprobe: tool("ffprobe")?,
            ffmpeg: tool("ffmpeg")?,
        })
    }

    /// The still frames of the video at `path`, as JPEG files: none when it can't be read,
    /// fewer when a run fails or runs out of time, and no more once `stop` is set. It runs
    /// the tools and waits for them: call it off the UI thread.
    fn grab(&self, path: &Path, stop: &StopFlag, limits: Limits) -> Vec<Vec<u8>> {
        if !path.is_absolute() {
            log::warn!("{} isn't an absolute path: no still frames", path.display());
            return Vec::new();
        }
        let started = Instant::now();
        let until = || (Instant::now() + limits.per_run).min(started + limits.in_all);
        let mut probe = Command::new(&self.ffprobe);
        probe
            .args(["-v", "error", "-show_entries", "format=duration", "-of"])
            .arg("default=noprint_wrappers=1:nokey=1")
            .arg(input(path));
        let duration = run(&mut probe, until(), stop)
            .and_then(|out| parse_duration(&String::from_utf8_lossy(&out)));
        let scale = format!(
            "scale='if(gt(iw,ih),min({LONGEST_SIDE},iw),-2)':'if(gt(iw,ih),-2,min({LONGEST_SIDE},ih))'"
        );
        let mut frames = Vec::new();
        for at in frame_times(duration.unwrap_or_default()) {
            let mut frame = Command::new(&self.ffmpeg);
            frame
                .args(["-v", "error", "-ss", &format!("{at:.3}"), "-i"])
                .arg(input(path))
                .args(["-frames:v", "1", "-vf", &scale, "-q:v", "3"])
                .args(["-f", "image2", "-c:v", "mjpeg", "pipe:1"]);
            match run(&mut frame, until(), stop) {
                Some(out) if !out.is_empty() => frames.push(out),
                Some(_) => log::warn!("ffmpeg made no frame of {} at {at:.3} s", path.display()),
                None if stop.is_set() || Instant::now() >= started + limits.in_all => break,
                None => {}
            }
        }
        frames
    }
}

/// Runs `command` until it ends, `until` comes or `stop` is set: what it printed, when it
/// ended well. One still running then is killed.
fn run(command: &mut Command, until: Instant, stop: &StopFlag) -> Option<Vec<u8>> {
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .inspect_err(|error| log::warn!("{program} didn't run: {error}"))
        .ok()?;
    // Read as it prints, so a full pipe never holds it up.
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout.read_to_end(&mut out).map(|_| out)
    });
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                return match reader.join() {
                    Ok(Ok(out)) => Some(out),
                    Ok(Err(error)) => {
                        log::warn!("{program}'s output didn't read: {error}");
                        None
                    }
                    Err(_) => None,
                };
            }
            Ok(Some(status)) => {
                log::warn!("{program} failed: {status}");
                return None;
            }
            Ok(None) if stop.is_set() || Instant::now() >= until => {
                log::warn!("{program} took too long or isn't wanted any more: stopped");
                end(&mut child, &program);
                return None;
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(error) => {
                log::warn!("{program} couldn't be waited for: {error}");
                end(&mut child, &program);
                return None;
            }
        }
    }
}

/// Kills `child` and reaps it.
fn end(child: &mut Child, program: &str) {
    if let Err(error) = child.kill() {
        log::debug!("{program} didn't take the kill: {error}");
    }
    if let Err(error) = child.wait() {
        log::debug!("{program} wasn't reaped: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffprobes_length_reads_as_seconds() {
        assert_eq!(parse_duration("12.480000\n"), Some(12.48));
        assert_eq!(parse_duration("N/A"), None);
        assert_eq!(parse_duration("0"), None);
    }

    #[test]
    fn only_absolute_folders_of_the_path_are_searched() {
        let path = std::env::join_paths(["/usr/bin", ".", "bin"]).ok();
        let found = folders(path);
        assert_eq!(found[0], PathBuf::from("/usr/bin"));
        assert!(found.iter().all(|folder| folder.is_absolute()), "{found:?}");
        assert_eq!(found.len(), 1 + FOLDERS.len());
    }

    #[test]
    fn a_file_goes_in_through_the_file_protocol() {
        assert_eq!(
            input(Path::new("/tmp/-a:b.mov")),
            OsString::from("file:/tmp/-a:b.mov")
        );
    }

    /// Fake `ffprobe` and `ffmpeg` scripts in a folder of their own.
    #[cfg(unix)]
    mod fake {
        use std::os::unix::fs::PermissionsExt as _;
        use std::path::Path;

        /// Writes the executable script `name` running `body`, its arguments one a line in
        /// `<name>.args` (appended).
        pub(super) fn tool(folder: &Path, name: &str, body: &str) {
            let script = folder.join(name);
            let log = folder.join(format!("{name}.args"));
            let text = format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\n{body}\n",
                log.display()
            );
            std::fs::write(&script, text).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        /// The arguments `name` was run with, one a line.
        pub(super) fn args(folder: &Path, name: &str) -> Vec<String> {
            std::fs::read_to_string(folder.join(format!("{name}.args")))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    #[cfg(unix)]
    fn tools(folder: &Path) -> Tools {
        Tools::find(&[folder.to_path_buf()]).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn frames_come_from_ffmpeg_at_their_times() {
        let dir = tempfile::tempdir().unwrap();
        fake::tool(dir.path(), "ffprobe", "echo 10.0");
        fake::tool(dir.path(), "ffmpeg", "printf 'jpeg'");
        let video = dir.path().join("clip.mov");
        let frames = tools(dir.path()).grab(&video, &StopFlag::default(), Limits::default());
        assert_eq!(frames, vec![b"jpeg".to_vec(); 4]);
        let probed = fake::args(dir.path(), "ffprobe");
        let file = format!("file:{}", video.display());
        assert_eq!(probed.last(), Some(&file));
        assert!(probed.contains(&"format=duration".to_owned()), "{probed:?}");
        let made = fake::args(dir.path(), "ffmpeg");
        let times: Vec<&str> = made
            .windows(2)
            .filter(|pair| pair[0] == "-ss")
            .map(|pair| pair[1].as_str())
            .collect();
        assert_eq!(times, ["0.500", "3.500", "6.500", "9.500"]);
        assert_eq!(made.iter().filter(|arg| **arg == file).count(), 4);
        assert!(made.contains(&"pipe:1".to_owned()));
    }

    #[cfg(unix)]
    #[test]
    fn a_video_ffmpeg_cant_read_has_no_frames() {
        let dir = tempfile::tempdir().unwrap();
        fake::tool(dir.path(), "ffprobe", "exit 1");
        fake::tool(dir.path(), "ffmpeg", "printf 'jpeg'");
        let video = dir.path().join("clip.mov");
        let none = tools(dir.path()).grab(&video, &StopFlag::default(), Limits::default());
        assert_eq!(none, Vec::<Vec<u8>>::new());
        assert!(
            fake::args(dir.path(), "ffmpeg").is_empty(),
            "no length, no frames"
        );
        fake::tool(dir.path(), "ffprobe", "echo 10.0");
        fake::tool(dir.path(), "ffmpeg", "exit 1");
        let failed = tools(dir.path()).grab(&video, &StopFlag::default(), Limits::default());
        assert_eq!(failed, Vec::<Vec<u8>>::new());
        assert!(
            tools(dir.path())
                .grab(
                    Path::new("clip.mov"),
                    &StopFlag::default(),
                    Limits::default()
                )
                .is_empty(),
            "a relative path isn't run"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_run_is_killed_and_the_video_goes_without_frames() {
        let dir = tempfile::tempdir().unwrap();
        fake::tool(dir.path(), "ffprobe", "echo 10.0");
        fake::tool(dir.path(), "ffmpeg", "exec sleep 30");
        let video = dir.path().join("clip.mov");
        let limits = Limits {
            per_run: Duration::from_millis(200),
            in_all: Duration::from_millis(500),
        };
        let started = Instant::now();
        let frames = tools(dir.path()).grab(&video, &StopFlag::default(), limits);
        assert_eq!(frames, Vec::<Vec<u8>>::new());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "back in {:?}, not after the 30 s the tool hangs",
            started.elapsed()
        );
        assert!(
            fake::args(dir.path(), "ffmpeg")
                .iter()
                .filter(|arg| *arg == "-ss")
                .count()
                < 4,
            "the time for all the runs ran out"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_tools_are_found_in_the_folders_given() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Tools::find(&[dir.path().to_path_buf()]), None);
        fake::tool(dir.path(), "ffprobe", "echo 1");
        assert_eq!(Tools::find(&[dir.path().to_path_buf()]), None, "ffmpeg too");
        fake::tool(dir.path(), "ffmpeg", "true");
        assert_eq!(
            Tools::find(&[dir.path().to_path_buf()]).map(|tools| tools.ffmpeg),
            Some(dir.path().join("ffmpeg"))
        );
    }
}
