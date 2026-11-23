//! Runs ffmpeg on a stream URL and reports progress.
//!
//! Knows nothing about sites or hosters. Needs `ffmpeg` and `ffprobe` on the `PATH`.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use kura_domain::{OutputFormat, Stream};
use tokio::{
    fs,
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    time::{Instant, sleep_until, timeout},
};

/// A download that writes nothing new for this long has stalled and is stopped.
pub const STALL_AFTER: Duration = Duration::from_secs(90);

/// ffmpeg gives up on a connection that sends nothing for this long, in microseconds.
/// Without it ffmpeg waits forever on a connection that went quiet. Shorter than
/// [`STALL_AFTER`], so ffmpeg's own error, which names the cause, usually comes first.
const READ_TIMEOUT_MICROS: &str = "30000000";

/// What ffmpeg logs when it cannot fetch a part of an HLS stream. It then leaves the part
/// out and goes on, so the file would have a gap.
const LOST_SEGMENT: &str = "Failed to open segment";

/// A file may end this far before the stated length, e.g. when the audio is a little shorter.
const MAY_MISS: f64 = 0.05;
const MAY_MISS_SECONDS: f64 = 30.0;

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
    /// The connection stayed open but no data arrived.
    #[error("the download stalled: nothing arrived for {0} seconds")]
    Stalled(u64),
    #[error("the stream ended early, at {0}% of its length")]
    Incomplete(u8),
    /// The hoster did not hand out a part in the middle, e.g. with a 404.
    #[error("a part of the stream could not be fetched")]
    MissingPart,
}

impl DownloadError {
    /// Whether the same episode may work with a freshly resolved stream: the stream failed,
    /// not this machine.
    pub fn is_stream_failure(&self) -> bool {
        matches!(
            self,
            Self::Ffmpeg(_) | Self::Stalled(_) | Self::Incomplete(_) | Self::MissingPart
        )
    }
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
/// Dropping the returned future stops ffmpeg and removes the partial file. A download that
/// stalls or ends early fails instead of hanging or leaving a short file.
pub async fn download(
    stream: &Stream,
    target: &Path,
    format: OutputFormat,
    on_progress: impl FnMut(Progress),
) -> Result<()> {
    download_through(stream, target, format, None, on_progress).await
}

/// Like [`download`], with every connection of ffmpeg and ffprobe going through the HTTP
/// proxy at `proxy` (e.g. `http://127.0.0.1:41234`): the first request, redirects and the
/// parts of an HLS playlist alike.
pub async fn download_through(
    stream: &Stream,
    target: &Path,
    format: OutputFormat,
    proxy: Option<&str>,
    on_progress: impl FnMut(Progress),
) -> Result<()> {
    // The URL comes from a hoster page; ffmpeg would also open local files and devices.
    if !stream.url.starts_with("https://") && !stream.url.starts_with("http://") {
        return Err(DownloadError::BadUrl(stream.url.clone()));
    }
    // Applies to the playlist and to every segment request.
    let mut input = vec!["-rw_timeout".to_owned(), READ_TIMEOUT_MICROS.to_owned()];
    if is_hls(&stream.url) {
        // Keeps the connection and opens the next part while the current one still loads, so
        // the download does not wait between parts. Only the HLS reader knows these options.
        input.extend(["-http_persistent", "1", "-http_multiple", "1"].map(String::from));
    }
    if !stream.headers.is_empty() {
        let headers = stream
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"));
        input.extend(["-headers".to_owned(), headers.collect()]);
    }
    if let Some(proxy) = proxy {
        // The HLS reader passes it on to every part it opens.
        input.extend(["-http_proxy".to_owned(), proxy.to_owned()]);
    }
    input.extend(["-i".to_owned(), stream.url.clone()]);
    run(&input, target, format, STALL_AFTER, on_progress).await
}

/// Whether the address names an HLS playlist, which ffmpeg loads part by part.
fn is_hls(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.to_ascii_lowercase().ends_with(".m3u8")
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
    stall_after: Duration,
    mut on_progress: impl FnMut(Progress),
) -> Result<()> {
    // ffmpeg cannot tell the container from the `.part` name, so it is named here.
    let container = match format {
        OutputFormat::Mkv => "matroska",
        OutputFormat::Mp4 => "mp4",
    };
    // ffprobe can hang on a quiet connection like ffmpeg; then the length stays unknown.
    let duration = timeout(stall_after, probe_duration(input))
        .await
        .unwrap_or(None);
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).await?;
    }
    let mut part = OsString::from(target);
    part.push(".part");
    let part = PartFile(part.into());

    let mut ffmpeg = Command::new("ffmpeg")
        // Warnings are read too: a part of the stream that could not be fetched is only one.
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "level+warning",
            "-y",
        ])
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
    let stderr = ffmpeg.stderr.take().expect("stderr was set to piped");
    let mut reports = BufReader::new(stdout).lines();
    let mut messages = BufReader::new(stderr).lines();
    let (mut reporting, mut messaging) = (true, true);

    let mut parser = ProgressParser::new(duration);
    let mut position = parser.position();
    let mut deadline = Instant::now() + stall_after;
    let mut error = None;
    // Both pipes are drained together, or ffmpeg stalls once one of them is full.
    let stopped = loop {
        tokio::select! {
            line = reports.next_line(), if reporting => match line {
                Ok(Some(line)) => {
                    if let Some(progress) = parser.line(&line) {
                        on_progress(progress);
                    }
                    // Newer ffmpeg versions keep reporting while they wait for data, so
                    // only a report that got further than the last one counts.
                    if parser.position() != position {
                        position = parser.position();
                        deadline = Instant::now() + stall_after;
                    }
                }
                _ => reporting = false,
            },
            line = messages.next_line(), if messaging => match line {
                Ok(Some(line)) if line.contains(LOST_SEGMENT) => {
                    break Some(DownloadError::MissingPart);
                }
                Ok(Some(line)) => {
                    if let Some(message) = error_message(&line) {
                        error = Some(message.to_owned());
                    }
                }
                _ => messaging = false,
            },
            () = sleep_until(deadline), if reporting || messaging => {
                break Some(DownloadError::Stalled(stall_after.as_secs()));
            }
            else => break None,
        }
    };
    if let Some(reason) = stopped {
        let _ = ffmpeg.start_kill();
        ffmpeg.wait().await?;
        return Err(reason);
    }

    let last = parser.current;
    if !ffmpeg.wait().await?.success() {
        let reason = error.unwrap_or_else(|| "no error message".to_owned());
        // ffmpeg sometimes fails only at the very end, e.g. on the last part or while
        // closing the file. A file that holds the whole stream is kept instead of being
        // downloaded again.
        if !holds_whole_stream(duration, last.fraction, &part.0).await {
            return Err(DownloadError::Ffmpeg(reason));
        }
        tracing::warn!("ffmpeg failed after writing the whole stream, keeping the file: {reason}");
    }
    if let Some(percent) = duration.and_then(|duration| ended_early(duration, last.fraction)) {
        return Err(DownloadError::Incomplete(percent));
    }
    fs::rename(&part.0, target).await?;
    Ok(())
}

/// The text of an error line of ffmpeg's log, without the `[error]` label. Warnings are
/// not errors.
fn error_message(line: &str) -> Option<&str> {
    ["[error] ", "[fatal] "].iter().find_map(|label| {
        let (_, message) = line.split_once(label)?;
        Some(message.trim())
    })
}

/// Whether the written file is as long as the stream: the stream states its length, ffmpeg
/// got to the end of it, and the file itself has about that length too.
async fn holds_whole_stream(duration: Option<f64>, fraction: f64, file: &Path) -> bool {
    let Some(duration) = duration else {
        return false;
    };
    if fraction <= 0.0 || ended_early(duration, fraction).is_some() {
        return false;
    }
    let input = ["-i".to_owned(), file.to_string_lossy().into_owned()];
    let written = probe_duration(&input).await;
    written.is_some_and(|written| ended_early(duration, written / duration).is_none())
}

/// The percentage a download reached, when it stopped well short of the stream's length.
fn ended_early(duration: f64, fraction: f64) -> Option<u8> {
    let missing = 1.0 - fraction;
    (missing > MAY_MISS && missing * duration > MAY_MISS_SECONDS)
        .then_some((fraction * 100.0) as u8)
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
    /// Microseconds of video written so far.
    written: i64,
}

impl ProgressParser {
    fn new(duration: Option<f64>) -> Self {
        Self {
            duration,
            current: Progress::default(),
            written: 0,
        }
    }

    /// How far the download is; unchanged for as long as nothing new was written.
    fn position(&self) -> (u64, i64) {
        (self.current.bytes, self.written)
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
                if let Ok(micros) = value.parse::<i64>() {
                    self.written = micros;
                    if let Some(duration) = self.duration {
                        self.current.fraction = (micros as f64 / 1e6 / duration).clamp(0.0, 1.0);
                    }
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
    use super::*;

    /// Longer than any test download takes.
    const PATIENT: Duration = Duration::from_secs(60);

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
            PATIENT,
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
            PATIENT,
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
            PATIENT,
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
            run(&slow, &target, OutputFormat::Mkv, PATIENT, |_| {}),
        )
        .await;

        assert!(started.is_err(), "the download finished: {started:?}");
        assert!(!target.exists());
        assert!(!dir.path().join("out.mkv.part").exists());
    }

    #[tokio::test]
    async fn a_download_that_gets_no_data_fails_instead_of_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.mkv");
        // A pipe that is open for writing but never written to: ffmpeg waits on it the way
        // it waits on a connection that went quiet.
        let pipe = dir.path().join("quiet");
        let made = std::process::Command::new("mkfifo").arg(&pipe).status();
        assert!(made.unwrap().success());
        let _writer = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe)
            .unwrap();

        let result = timeout(
            Duration::from_secs(20),
            run(
                &input(&pipe),
                &target,
                OutputFormat::Mkv,
                Duration::from_millis(300),
                |_| {},
            ),
        )
        .await
        .expect("the stalled download was stopped");

        assert!(
            matches!(result, Err(DownloadError::Stalled(_))),
            "{result:?}"
        );
        assert!(!target.exists());
        assert!(!dir.path().join("out.mkv.part").exists());
    }

    #[test]
    fn only_hls_playlists_get_the_hls_options() {
        assert!(is_hls(
            "https://cdn.example/hls/abc/master.m3u8?t=token&s=1"
        ));
        assert!(is_hls("https://cdn.example/index.M3U8"));
        assert!(!is_hls("https://cdn.example/video.mp4?file=a.m3u8"));
        assert!(!is_hls("https://cdn.example/video.mp4"));
    }

    #[tokio::test]
    async fn an_hls_stream_downloads_with_the_hls_options() {
        let dir = tempfile::tempdir().unwrap();
        let playlist = dir.path().join("list.m3u8");
        let made = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-f", "lavfi", "-i"])
            .args([
                "testsrc=duration=4:size=64x64:rate=10",
                "-c:v",
                "mpeg2video",
            ])
            .args(["-f", "hls", "-hls_time", "1", "-hls_list_size", "0"])
            .arg(&playlist)
            .status()
            .expect("the tests need ffmpeg");
        assert!(made.success());
        let target = dir.path().join("out.mkv");
        let mut options = ["-http_persistent", "1", "-http_multiple", "1"]
            .map(String::from)
            .to_vec();
        options.extend(input(&playlist));

        run(&options, &target, OutputFormat::Mkv, PATIENT, |_| {})
            .await
            .unwrap();
        let length = probe_duration(&input(&target)).await.unwrap();
        assert!(length > 3.5, "{length}");
    }

    #[tokio::test]
    async fn a_file_is_kept_after_a_late_error_only_when_it_holds_the_whole_stream() {
        let dir = tempfile::tempdir().unwrap();
        let file = sample(dir.path());
        // The sample is two seconds long.
        assert!(holds_whole_stream(Some(2.0), 1.0, &file).await);
        // ffmpeg did not get to the end, or the length is unknown.
        assert!(!holds_whole_stream(Some(1440.0), 0.5, &file).await);
        assert!(!holds_whole_stream(None, 1.0, &file).await);
        // ffmpeg reported the end, but the file is far shorter than the stream.
        assert!(!holds_whole_stream(Some(1440.0), 1.0, &file).await);
        assert!(!holds_whole_stream(Some(2.0), 1.0, &dir.path().join("missing.mkv")).await);
    }

    #[test]
    fn a_download_that_ends_well_before_the_stated_length_ended_early() {
        // A 24-minute episode that stopped at 60%.
        assert_eq!(ended_early(1440.0, 0.6), Some(60));
        // The last seconds may be missing, and a short clip may miss a larger share.
        assert_eq!(ended_early(1440.0, 0.99), None);
        assert_eq!(ended_early(20.0, 0.5), None);
    }

    #[tokio::test]
    async fn a_stream_with_a_missing_part_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.mkv");
        let segment = dir.path().join("part0.ts");
        let made = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-f", "lavfi", "-i"])
            .args([
                "testsrc=duration=2:size=64x64:rate=10",
                "-c:v",
                "mpeg2video",
            ])
            .arg(&segment)
            .status()
            .unwrap();
        assert!(made.success());
        // The second part does not exist, the way a hoster answers 404 for it.
        let playlist = dir.path().join("index.m3u8");
        std::fs::write(
            &playlist,
            "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\npart0.ts\n#EXTINF:2,\npart1.ts\n\
             #EXTINF:2,\npart0.ts\n#EXT-X-ENDLIST\n",
        )
        .unwrap();

        let result = run(
            &input(&playlist),
            &target,
            OutputFormat::Mkv,
            PATIENT,
            |_| {},
        )
        .await;

        assert!(
            matches!(result, Err(DownloadError::MissingPart)),
            "{result:?}"
        );
        assert!(!target.exists());
        assert!(!dir.path().join("out.mkv.part").exists());
    }

    #[test]
    fn only_error_lines_of_the_log_are_errors() {
        assert_eq!(
            error_message(
                "[in#0 @ 0x55] [error] Error opening input: Server returned 404 Not Found"
            ),
            Some("Error opening input: Server returned 404 Not Found")
        );
        assert_eq!(
            error_message("[fatal] Error opening input files: Connection timed out"),
            Some("Error opening input files: Connection timed out")
        );
        assert_eq!(
            error_message("[http @ 0x55] [warning] HTTP error 404 Not Found"),
            None
        );
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
