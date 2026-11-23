use super::components::{Mode, tools};

pub fn user_error(error: &anyhow::Error) -> String {
    user_error_for(error, tools().mode)
}

fn user_error_for(error: &anyhow::Error, mode: Mode) -> String {
    let detail = format!("{error:#}");
    let lower = detail.to_lowercase();
    if lower.contains("timed out") {
        error.to_string()
    } else if lower.contains("403") {
        "YouTube denied the audio stream (HTTP 403). Try an accessible video.".into()
    } else if lower.contains("sign in")
        || lower.contains("not a bot")
        || lower.contains("private video")
        || lower.contains("unavailable")
        || lower.contains("age") && lower.contains("restrict")
    {
        "YouTube restricted this video. Try a public video that permits playback.".into()
    } else if lower.contains("ejs") || lower.contains("challenge") || lower.contains("javascript") {
        if mode == Mode::Managed {
            "YouTube challenge failed. Update Grajek to the latest version; check the logs.".into()
        } else {
            "YouTube challenge failed. Update yt-dlp, Deno and EJS; check the logs.".into()
        }
    } else if lower.contains("yt-dlp extraction failed") {
        if mode == Mode::Managed {
            "YouTube extraction failed. Check for a newer Grajek release; check the logs.".into()
        } else {
            "YouTube extraction failed. Check the logs and update yt-dlp.".into()
        }
    } else {
        error.to_string()
    }
}
