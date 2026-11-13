//! Runs ffmpeg on a stream URL and reports progress.
//!
//! Knows nothing about sites or hosters. Needs `ffmpeg` and `ffprobe` on the `PATH`.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
};

use kura_domain::{OutputFormat, Stream};
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
};

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("not a web address: {0}")]
    BadUrl(String),
    #[error("could not run {0}: {1}")]
    Spawn(&'static str, std::io::Error),
    #[error("could not write the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
}

pub type Result<T> = std::result::Result<T, DownloadError>;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Progress {
    /// 0.0 to 1.0; stays 0.0 when the stream does not state its length.
    pub fraction: f64,
    /// Bytes written so far.
    pub bytes: u64,
}

/// Downloads `stream` to `target` as Matroska without re-encoding.
///
/// The data goes to `<target>.part` first, so `target` only ever holds a complete file.
/// Dropping the returned future stops ffmpeg and removes the partial file.
pub async fn download(
    stream: &Stream,
    target: &Path,
    format: OutputFormat,
    on_progress: impl FnMut(Progress),
) -> Result<()> {
    // The URL comes from a hoster page; ffmpeg would also open local files and devices.
    if !stream.url.starts_with("https://") && !stream.url.starts_with("http://") {
        return Err(DownloadError::BadUrl(stream.url.clone()));
    }
    let mut input = Vec::new();
    if !stream.headers.is_empty() {
        let headers = stream
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"));
        input.extend(["-headers".to_owned(), headers.collect()]);
    }
    input.extend(["-i".to_owned(), stream.url.clone()]);
    run(&input, target, format, on_progress).await
}

/// Removes the partial file when a download fails or is dropped. After a successful rename
/// there is nothing left to remove.
struct PartFile(PathBuf);

impl Drop for PartFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn run(
    input: &[String],
    target: &Path,
    format: OutputFormat,
    mut on_progress: impl FnMut(Progress),
) -> Result<()> {
    // ffmpeg cannot tell the container from the `.part` name, so it is named here.
    let container = match format {
        OutputFormat::Mkv => "matroska",
        OutputFormat::Mp4 => "mp4",
    };
    let duration = probe_duration(input).await;
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).await?;
    }
    let mut part = OsString::from(target);
    part.push(".part");
    let part = PartFile(part.into());

    let mut ffmpeg = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
        .args(["-progress", "pipe:1", "-nostats"])
        .args(input)
        .args(["-c", "copy", "-f", container])
        .arg(&part.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| DownloadError::Spawn("ffmpeg", err))?;
    let stdout = ffmpeg.stdout.take().expect("stdout was set to piped");
    let mut stderr = ffmpeg.stderr.take().expect("stderr was set to piped");

    let report = async {
        let mut parser = ProgressParser::new(duration);
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(progress) = parser.line(&line) {
                on_progress(progress);
            }
        }
    };
    // Both pipes are drained together, or ffmpeg stalls once one of them is full.
    let mut errors = String::new();
    let _ = tokio::join!(report, stderr.read_to_string(&mut errors));

    if !ffmpeg.wait().await?.success() {
        let reason = errors.lines().last().unwrap_or("no error message");
        return Err(DownloadError::Ffmpeg(reason.to_owned()));
    }
    fs::rename(&part.0, target).await?;
    Ok(())
}

/// Length of the input in seconds, if it states one.
async fn probe_duration(input: &[String]) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args(["-v", "error"])
        .args(input)
        .args(["-show_entries", "format=duration", "-of", "csv=p=0"])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    let seconds: f64 = String::from_utf8(output.stdout).ok()?.trim().parse().ok()?;
    (seconds > 0.0).then_some(seconds)
}

/// Reads the `key=value` lines of `ffmpeg -progress`. A report ends with a `progress=` line.
struct ProgressParser {
    duration: Option<f64>,
    current: Progress,
}

impl ProgressParser {
    fn new(duration: Option<f64>) -> Self {
        Self {
            duration,
            current: Progress::default(),
        }
    }

    fn line(&mut self, line: &str) -> Option<Progress> {
        let (key, value) = line.split_once('=')?;
        match key {
            "total_size" => {
                if let Ok(bytes) = value.parse() {
                    self.current.bytes = bytes;
                }
            }
            // "N/A" until the first packet is written.
            "out_time_us" => {
                if let (Ok(micros), Some(duration)) = (value.parse::<f64>(), self.duration) {
                    self.current.fraction = (micros / 1e6 / duration).clamp(0.0, 1.0);
                }
            }
            "progress" => return Some(self.current),
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn progress_lines_become_reports() {
        let output = "\
frame=0
total_size=N/A
out_time_us=N/A
speed=N/A
progress=continue
frame=120
bitrate=1677.7kbits/s
total_size=1048576
out_time_us=5000000
out_time=00:00:05.000000
speed=2.0x
progress=continue
total_size=4194304
out_time_us=20100000
progress=end
";
        let mut parser = ProgressParser::new(Some(20.0));
        let reports: Vec<_> = output
            .lines()
            .filter_map(|line| parser.line(line))
            .collect();
        assert_eq!(
            reports,
            [
                Progress {
                    fraction: 0.0,
                    bytes: 0
                },
                Progress {
                    fraction: 0.25,
                    bytes: 1_048_576
                },
                Progress {
                    fraction: 1.0,
                    bytes: 4_194_304
                },
            ]
        );

        let mut unknown_length = ProgressParser::new(None);
        let last = output
            .lines()
            .filter_map(|line| unknown_length.line(line))
            .last();
        assert_eq!(
            last,
            Some(Progress {
                fraction: 0.0,
                bytes: 4_194_304
            })
        );
    }

    fn input(path: &Path) -> Vec<String> {
        vec!["-i".to_owned(), path.to_str().unwrap().to_owned()]
    }

    /// A two-second test video made by ffmpeg itself.
    fn sample(dir: &Path) -> PathBuf {
        let path = dir.join("sample.mkv");
        let made = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-f", "lavfi", "-i"])
            .args(["testsrc=duration=2:size=64x64:rate=10", "-c:v", "mpeg4"])
            .arg(&path)
            .status()
            .expect("the tests need ffmpeg");
        assert!(made.success());
        path
    }

    #[tokio::test]
    async fn downloads_to_the_target_and_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Series/Season 01/Series - S01E01.mkv");
        let mut reports = Vec::new();

        run(
            &input(&sample(dir.path())),
            &target,
            OutputFormat::Mkv,
            |progress| reports.push(progress),
        )
        .await
        .unwrap();

        let last = reports.last().unwrap();
        assert!(last.fraction > 0.9, "{last:?}");
        assert_eq!(last.bytes, std::fs::metadata(&target).unwrap().len());
        assert!(!target.with_file_name("Series - S01E01.mkv.part").exists());
    }

    #[tokio::test]
    async fn writes_the_chosen_container() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.mp4");
        run(
            &input(&sample(dir.path())),
            &target,
            OutputFormat::Mp4,
            |_| {},
        )
        .await
        .unwrap();

        let probe = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "format=format_name"])
            .args(["-of", "csv=p=0"])
            .arg(&target)
            .output()
            .unwrap();
        let container = String::from_utf8(probe.stdout).unwrap();
        assert!(container.contains("mp4"), "{container}");
    }

    #[tokio::test]
    async fn a_failed_download_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.mkv");

        let result = run(
            &input(&dir.path().join("missing.mkv")),
            &target,
            OutputFormat::Mkv,
            |_| {},
        )
        .await;

        assert!(
            matches!(result, Err(DownloadError::Ffmpeg(_))),
            "{result:?}"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn dropping_a_download_stops_it_and_removes_the_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.mkv");
        // `-re` reads at playback speed, so the two-second sample takes two seconds.
        let mut slow = vec!["-re".to_owned()];
        slow.extend(input(&sample(dir.path())));

        let started = tokio::time::timeout(
            Duration::from_millis(500),
            run(&slow, &target, OutputFormat::Mkv, |_| {}),
        )
        .await;

        assert!(started.is_err(), "the download finished: {started:?}");
        assert!(!target.exists());
        assert!(!dir.path().join("out.mkv.part").exists());
    }

    #[tokio::test]
    async fn only_web_addresses_are_downloaded() {
        let stream = Stream {
            url: "file:///etc/passwd".into(),
            headers: Vec::new(),
        };
        let result = download(&stream, Path::new("out.mkv"), OutputFormat::Mkv, |_| {}).await;
        assert!(matches!(result, Err(DownloadError::BadUrl(_))));
    }
}
